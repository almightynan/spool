// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Application start-up, actions and keyboard shortcuts.

use super::context::{apply_style, AppContext};
use crate::runtime::Worker;
use crate::ui::window::MainWindow;
use adw::prelude::*;
use spool_services::cups::CupsClient;
use spool_services::discovery::{spawn_registry, Registry};
use spool_services::hplip::{self, HplipAvailability};
use spool_services::identity::current_user;
use spool_services::poll::PollControl;
use spool_services::settings::{self, Settings};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

pub const APP_ID: &str = "io.github.almightynan.Spool";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Everything start-up needs from the worker (file and socket IO).
struct Init {
    cups: Arc<CupsClient>,
    registry: Registry,
    settings: Settings,
    settings_path: Option<PathBuf>,
    settings_error: Option<String>,
    hplip: HplipAvailability,
    pictures_dir: Option<PathBuf>,
}

async fn init(poll: PollControl) -> Init {
    // Without a known name, use the identity RFC 8011 section 9.3 gives
    // unauthenticated users: "anonymous".
    let user = current_user().await.unwrap_or_else(|| "anonymous".into());
    let cups = Arc::new(CupsClient::new(user));
    let settings_path = settings::default_path();
    let (settings, settings_error) = match &settings_path {
        Some(p) => match Settings::load(p).await {
            Ok(s) => (s, None),
            Err(e) => (Settings::default(), Some(e.to_string())),
        },
        None => (Settings::default(), None),
    };
    poll.set_interval(settings.refresh_interval());
    let hplip = hplip::detect().await;
    let pictures_dir = gtk::glib::user_special_dir(gtk::glib::UserDirectory::Pictures);
    let registry = spawn_registry(
        &tokio::runtime::Handle::current(),
        cups.clone(),
        &poll,
        settings.manual_hosts.clone(),
    );
    Init {
        cups,
        registry,
        settings,
        settings_path,
        settings_error,
        hplip,
        pictures_dir,
    }
}

pub fn run() -> gtk::glib::ExitCode {
    let worker = match Worker::new() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("spool: cannot start worker threads: {e}");
            return gtk::glib::ExitCode::FAILURE;
        }
    };
    let worker = Rc::new(RefCell::new(Some(worker)));
    let app = adw::Application::builder().application_id(APP_ID).build();
    let window: Rc<RefCell<Option<Rc<MainWindow>>>> = Rc::new(RefCell::new(None));

    app.connect_startup(|app| {
        // The interface uses Adwaita symbolic icons. Other themes (Breeze on
        // KDE) lack several of them, and GTK then falls back to color icons.
        // GtkSettings carries the theme name from settings.ini, so it is set
        // there; setting it on the IconTheme alone is overridden.
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_icon_theme_name(Some("Adwaita"));
        }
        app.set_accels_for_action("app.quit", &["<Control>q"]);
        app.set_accels_for_action("app.preferences", &["<Control>comma"]);
        app.set_accels_for_action("win.add-device", &["<Control>n"]);
        app.set_accels_for_action("win.refresh", &["<Control>r", "F5"]);
        app.set_accels_for_action("window.close", &["<Control>w"]);
        let quit = gtk::gio::SimpleAction::new("quit", None);
        quit.connect_activate({
            let app = app.clone();
            move |_, _| app.quit()
        });
        app.add_action(&quit);
    });

    app.connect_activate({
        let window = window.clone();
        move |app| {
            if let Some(w) = window.borrow().as_ref() {
                w.window.present();
                return;
            }
            let Some(worker) = worker.borrow_mut().take() else {
                return;
            };
            let w = MainWindow::new(app);
            w.window.present();
            *window.borrow_mut() = Some(w.clone());
            start(app, worker, w);
        }
    });
    app.run()
}

fn start(app: &adw::Application, worker: Worker, w: Rc<MainWindow>) {
    let poll = PollControl::new(std::time::Duration::from_secs(u64::from(
        settings::REFRESH_DEFAULT,
    )));
    let fut = worker.call(init(poll.clone()));
    let app = app.clone();
    gtk::glib::spawn_future_local(async move {
        let Ok(init) = fut.await else { return };
        apply_style(init.settings.style);
        let ctx = Rc::new(AppContext {
            worker,
            cups: init.cups,
            poll,
            registry: Arc::new(init.registry),
            settings: RefCell::new(init.settings),
            settings_path: init.settings_path,
            hplip: init.hplip,
            pictures_dir: init.pictures_dir,
            window: w.window.clone(),
            toasts: w.toasts.clone(),
            save_timer: RefCell::new(None),
            save_lock: Arc::new(tokio::sync::Mutex::new(())),
        });
        if let Some(e) = init.settings_error {
            ctx.toast(&format!("Could not read settings, using defaults: {e}"));
        }
        add_app_actions(&app, &ctx);
        w.attach(ctx);
    });
}

fn add_app_actions(app: &adw::Application, ctx: &Rc<AppContext>) {
    let prefs = gtk::gio::SimpleAction::new("preferences", None);
    prefs.connect_activate({
        let ctx = ctx.clone();
        move |_, _| crate::ui::preferences::present(&ctx)
    });
    app.add_action(&prefs);

    let about = gtk::gio::SimpleAction::new("about", None);
    about.connect_activate({
        let ctx = ctx.clone();
        move |_, _| {
            let d = adw::AboutDialog::builder()
                .application_name("Spool")
                .application_icon(APP_ID)
                .version(VERSION)
                .comments("Manage network printers and scanners with IPP, eSCL and CUPS.")
                .license_type(gtk::License::Gpl30)
                .developer_name("Spool contributors")
                .build();
            d.present(Some(&ctx.window));
        }
    });
    app.add_action(&about);
}
