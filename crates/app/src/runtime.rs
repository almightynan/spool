// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! The worker runtime and its bridge to the GTK main loop.
//!
//! Every network, file and process operation runs on this tokio runtime. The
//! UI awaits results through oneshot and watch channels from
//! `glib::spawn_future_local`, so the GTK thread never blocks.

use std::future::Future;
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::sync::{oneshot, watch};

pub struct Worker {
    runtime: Option<Runtime>,
}

/// The worker dropped the task (only during shutdown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped;

impl Worker {
    pub fn new() -> std::io::Result<Self> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("spool-worker")
            .enable_all()
            .build()?;
        Ok(Self {
            runtime: Some(runtime),
        })
    }

    /// Handle for services that spawn their own tasks.
    pub fn handle(&self) -> Handle {
        match &self.runtime {
            Some(rt) => rt.handle().clone(),
            // Only reachable after Drop started, when nothing calls this.
            None => Handle::current(),
        }
    }

    /// Runs `fut` on the worker. Await the returned future on the GTK main loop.
    pub fn call<F, T>(&self, fut: F) -> impl Future<Output = Result<T, Stopped>> + 'static
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        if let Some(rt) = &self.runtime {
            rt.spawn(async move {
                let _ = tx.send(fut.await);
            });
        }
        async move { rx.await.map_err(|_| Stopped) }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Do not wait for running requests when the app quits.
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

/// Calls `f` on the GTK main loop with every new value of `rx`, starting with
/// the current one. Ends when the sender is gone or `f` returns false.
pub fn follow<T, F>(mut rx: watch::Receiver<T>, mut f: F)
where
    T: Clone + 'static,
    F: FnMut(T) -> bool + 'static,
{
    gtk::glib::spawn_future_local(async move {
        let first = rx.borrow_and_update().clone();
        if !f(first) {
            return;
        }
        while rx.changed().await.is_ok() {
            let v = rx.borrow_and_update().clone();
            if !f(v) {
                return;
            }
        }
    });
}
