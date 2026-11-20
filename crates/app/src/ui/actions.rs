// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Running one user action on the worker with its button showing progress.

use super::widgets::BusyButton;
use crate::app::context::AppContext;
use std::future::Future;
use std::rc::Rc;

/// Marks `button` busy, runs `fut` on the worker and calls `done` with the
/// result on the GTK thread. Ignored while the button is already busy, so
/// one button never has two operations in flight.
pub fn run<T, F, D>(ctx: &Rc<AppContext>, button: Option<&BusyButton>, fut: F, done: D)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    D: FnOnce(T) + 'static,
{
    if button.is_some_and(BusyButton::is_busy) {
        return;
    }
    let button = button.cloned();
    if let Some(b) = &button {
        b.set_busy(true);
    }
    let call = ctx.worker.call(fut);
    gtk::glib::spawn_future_local(async move {
        let r = call.await;
        if let Some(b) = button {
            b.set_busy(false);
        }
        if let Ok(r) = r {
            done(r);
        }
    });
}
