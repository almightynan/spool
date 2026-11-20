// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared polling schedule. The UI reports window visibility; pollers wait on
//! it. Hidden windows stop polling, unfocused windows poll four times slower,
//! and a window that becomes visible again gets fresh data at once.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Notify};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Focused,
    Unfocused,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollState {
    pub interval: Duration,
    pub visibility: Visibility,
}

pub const UNFOCUSED_FACTOR: u32 = 4;

impl PollState {
    /// Time until the next poll, or None while polling is paused.
    pub fn effective(&self) -> Option<Duration> {
        match self.visibility {
            Visibility::Focused => Some(self.interval),
            Visibility::Unfocused => Some(self.interval * UNFOCUSED_FACTOR),
            Visibility::Hidden => None,
        }
    }
}

/// Owned by the app; cheap to clone.
#[derive(Clone)]
pub struct PollControl {
    tx: Arc<watch::Sender<PollState>>,
}

impl PollControl {
    pub fn new(interval: Duration) -> Self {
        let (tx, _) = watch::channel(PollState {
            interval,
            visibility: Visibility::Focused,
        });
        Self { tx: Arc::new(tx) }
    }

    pub fn set_visibility(&self, v: Visibility) {
        self.tx.send_if_modified(|s| {
            let changed = s.visibility != v;
            s.visibility = v;
            changed
        });
    }

    pub fn set_interval(&self, d: Duration) {
        self.tx.send_if_modified(|s| {
            let changed = s.interval != d;
            s.interval = d;
            changed
        });
    }

    pub fn state(&self) -> PollState {
        *self.tx.borrow()
    }

    pub fn waiter(&self) -> PollWaiter {
        PollWaiter {
            rx: self.tx.subscribe(),
        }
    }
}

pub struct PollWaiter {
    rx: watch::Receiver<PollState>,
}

impl PollWaiter {
    /// Waits until the next poll is due. Returns false when the control was
    /// dropped (the app is shutting down).
    pub async fn wait(&mut self, refresh: &Notify) -> bool {
        let mut was_paused = false;
        loop {
            let st = *self.rx.borrow_and_update();
            match st.effective() {
                None => {
                    was_paused = true;
                    tokio::select! {
                        r = self.rx.changed() => if r.is_err() { return false },
                        _ = refresh.notified() => return true,
                    }
                }
                Some(_) if was_paused => return true,
                Some(d) => {
                    tokio::select! {
                        _ = tokio::time::sleep(d) => return true,
                        r = self.rx.changed() => if r.is_err() { return false },
                        _ = refresh.notified() => return true,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn interval_focus_and_pause() {
        let c = PollControl::new(Duration::from_secs(10));
        let mut w = c.waiter();
        let refresh = Notify::new();

        let t0 = tokio::time::Instant::now();
        assert!(w.wait(&refresh).await);
        assert_eq!(t0.elapsed(), Duration::from_secs(10));

        c.set_visibility(Visibility::Unfocused);
        let t0 = tokio::time::Instant::now();
        assert!(w.wait(&refresh).await);
        assert_eq!(t0.elapsed(), Duration::from_secs(40));

        // Hidden: nothing happens for an hour, then becoming visible polls at once.
        c.set_visibility(Visibility::Hidden);
        let c2 = c.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            c2.set_visibility(Visibility::Focused);
        });
        let t0 = tokio::time::Instant::now();
        assert!(w.wait(&refresh).await);
        assert_eq!(t0.elapsed(), Duration::from_secs(3600));
    }

    #[tokio::test(start_paused = true)]
    async fn refresh_wakes_early_and_drop_stops() {
        let c = PollControl::new(Duration::from_secs(10));
        let mut w = c.waiter();
        let refresh = Notify::new();
        refresh.notify_one();
        let t0 = tokio::time::Instant::now();
        assert!(w.wait(&refresh).await);
        assert_eq!(t0.elapsed(), Duration::ZERO);
        drop(c);
        assert!(!w.wait(&refresh).await);
    }

    #[tokio::test(start_paused = true)]
    async fn interval_change_restarts_wait() {
        let c = PollControl::new(Duration::from_secs(60));
        let mut w = c.waiter();
        let refresh = Notify::new();
        let c2 = c.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            c2.set_interval(Duration::from_secs(5));
        });
        let t0 = tokio::time::Instant::now();
        assert!(w.wait(&refresh).await);
        assert_eq!(t0.elapsed(), Duration::from_secs(6));
    }
}

/// Latest result of a poller.
#[derive(Debug, Clone, PartialEq)]
pub enum Polled<T, E> {
    Loading,
    Ready(T),
    Failed(E),
}

/// Runs `fetch` now and then on the [`PollControl`] schedule, publishing each
/// result. Stops when the handle is dropped.
pub struct PollerHandle<T, E> {
    pub state: watch::Receiver<Polled<T, E>>,
    refresh: Arc<Notify>,
    cancel: crate::net::CancelToken,
    task: tokio::task::JoinHandle<()>,
}

impl<T, E> PollerHandle<T, E> {
    /// Polls now instead of waiting for the next interval.
    pub fn refresh(&self) {
        self.refresh.notify_one();
    }
}

impl<T, E> Drop for PollerHandle<T, E> {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

pub fn spawn_poller<T, E, F, Fut>(
    rt: &tokio::runtime::Handle,
    poll: &PollControl,
    fetch: F,
) -> PollerHandle<T, E>
where
    T: Send + Sync + 'static,
    E: Send + Sync + 'static,
    F: Fn(crate::net::CancelToken) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send,
{
    let (tx, rx) = watch::channel(Polled::Loading);
    let refresh = Arc::new(Notify::new());
    let cancel = crate::net::CancelToken::new();
    let mut waiter = poll.waiter();
    let task = rt.spawn({
        let refresh = refresh.clone();
        let cancel = cancel.clone();
        async move {
            loop {
                let next = match fetch(cancel.clone()).await {
                    Ok(v) => Polled::Ready(v),
                    Err(e) => Polled::Failed(e),
                };
                if cancel.is_cancelled() || tx.send(next).is_err() {
                    return;
                }
                if !waiter.wait(&refresh).await {
                    return;
                }
            }
        }
    });
    PollerHandle {
        state: rx,
        refresh,
        cancel,
        task,
    }
}

#[cfg(test)]
mod poller_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test(start_paused = true)]
    async fn publishes_and_stops_on_drop() {
        let poll = PollControl::new(Duration::from_secs(5));
        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        let mut h = spawn_poller(&tokio::runtime::Handle::current(), &poll, move |_cancel| {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n == 1 {
                    Err("down")
                } else {
                    Ok(n)
                }
            }
        });
        h.state.changed().await.unwrap();
        assert_eq!(*h.state.borrow_and_update(), Polled::Ready(0));
        h.state.changed().await.unwrap();
        assert_eq!(*h.state.borrow_and_update(), Polled::Failed("down"));
        h.refresh();
        h.state.changed().await.unwrap();
        assert_eq!(*h.state.borrow_and_update(), Polled::Ready(2));
        drop(h);
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }
}
