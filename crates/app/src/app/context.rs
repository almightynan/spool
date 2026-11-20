// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared state for the running app. Lives on the GTK thread (Rc).

use crate::runtime::Worker;
use spool_services::cups::CupsClient;
use spool_services::discovery::Registry;
use spool_services::hplip::HplipAvailability;
use spool_services::poll::PollControl;
use spool_services::settings::{Settings, Style};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

pub struct AppContext {
    pub worker: Worker,
    pub cups: Arc<CupsClient>,
    pub poll: PollControl,
    pub registry: Arc<Registry>,
    pub settings: RefCell<Settings>,
    pub settings_path: Option<PathBuf>,
    pub hplip: HplipAvailability,
    /// The user's Pictures folder, resolved on the worker at start.
    pub pictures_dir: Option<PathBuf>,
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    /// Pending debounced settings save.
    pub save_timer: RefCell<Option<gtk::glib::SourceId>>,
    /// Serializes settings writes on the worker.
    pub save_lock: Arc<tokio::sync::Mutex<()>>,
}

/// Settings changes within this time are written once.
const SAVE_DELAY_MS: u64 = 500;

impl AppContext {
    pub fn toast(&self, text: &str) {
        let t = adw::Toast::new(text);
        t.set_timeout(5);
        self.toasts.add_toast(t);
    }

    pub fn toast_with_action(&self, text: &str, label: &str, f: impl Fn() + 'static) {
        let t = adw::Toast::new(text);
        t.set_button_label(Some(label));
        t.set_timeout(8);
        t.connect_button_clicked(move |_| f());
        self.toasts.add_toast(t);
    }

    /// Folder offered when saving scans.
    pub fn save_folder(&self) -> Option<PathBuf> {
        self.settings
            .borrow()
            .save_folder
            .clone()
            .or_else(|| self.pictures_dir.clone())
    }

    /// Changes settings, applies them at once and writes the file shortly
    /// after on the worker.
    pub fn update_settings(self: &Rc<Self>, f: impl FnOnce(&mut Settings)) {
        let snapshot = {
            let mut s = self.settings.borrow_mut();
            f(&mut s);
            s.clone()
        };
        self.poll.set_interval(snapshot.refresh_interval());
        apply_style(snapshot.style);
        if let Some(t) = self.save_timer.borrow_mut().take() {
            t.remove();
        }
        let weak = Rc::downgrade(self);
        let id = gtk::glib::timeout_add_local_once(
            std::time::Duration::from_millis(SAVE_DELAY_MS),
            move || {
                if let Some(ctx) = weak.upgrade() {
                    *ctx.save_timer.borrow_mut() = None;
                    ctx.save_now();
                }
            },
        );
        *self.save_timer.borrow_mut() = Some(id);
    }

    fn save_now(self: &Rc<Self>) {
        let Some(path) = self.settings_path.clone() else {
            self.toast("Settings cannot be saved because no configuration folder was found");
            return;
        };
        let snapshot = self.settings.borrow().clone();
        let lock = self.save_lock.clone();
        let fut = self.worker.call(async move {
            let _guard = lock.lock().await;
            snapshot.save(&path).await
        });
        let ctx = self.clone();
        gtk::glib::spawn_future_local(async move {
            if let Ok(Err(e)) = fut.await {
                ctx.toast(&format!("Could not save settings: {e}"));
            }
        });
    }
}

pub fn apply_style(style: Style) {
    adw::StyleManager::default().set_color_scheme(match style {
        Style::System => adw::ColorScheme::Default,
        Style::Light => adw::ColorScheme::ForceLight,
        Style::Dark => adw::ColorScheme::ForceDark,
    });
}
