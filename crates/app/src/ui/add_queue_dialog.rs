// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! "Add Printer": creates a CUPS queue with the IPP Everywhere driver.

use super::actions;
use super::text;
use super::widgets::BusyButton;
use super::window::MainWindow;
use crate::app::context::AppContext;
use adw::prelude::*;
use spool_protocols::cups::{is_valid_queue_name, queue_name_from};
use spool_services::cups::NewQueue;
use spool_services::model::{Device, DeviceId};
use spool_services::net::CancelToken;
use std::rc::Rc;

pub fn present(ctx: &Rc<AppContext>, win: &Rc<MainWindow>, device: &Device) {
    let Some(device_uri) = device.printer_uri.clone() else {
        ctx.toast("This device has no IPP printer to add");
        return;
    };
    let base = device
        .make_and_model
        .clone()
        .unwrap_or_else(|| device.name.clone());
    let name = adw::EntryRow::builder()
        .title("Queue name")
        .text(queue_name_from(&base))
        .build();
    let info = adw::EntryRow::builder()
        .title("Description")
        .text(&device.name)
        .build();
    let location = adw::EntryRow::builder()
        .title("Location")
        .text(device.location.as_deref().unwrap_or_default())
        .build();
    let group = adw::PreferencesGroup::builder()
        .description(format!(
            "Prints go to {device_uri} with the driverless IPP Everywhere driver."
        ))
        .build();
    group.add(&name);
    group.add(&info);
    group.add(&location);

    let error = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["error"])
        .visible(false)
        .build();
    error.set_accessible_role(gtk::AccessibleRole::Alert);
    let progress = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .label("CUPS is reading the printer's capabilities to set up the driver. This can take up to a minute.")
        .visible(false)
        .build();

    let add = BusyButton::new("_Add");
    add.widget().add_css_class("suggested-action");
    let cancel = gtk::Button::with_mnemonic("_Cancel");
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(add.widget());

    let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
    col.set_margin_top(12);
    col.set_margin_bottom(24);
    col.set_margin_start(12);
    col.set_margin_end(12);
    col.append(&group);
    col.append(&error);
    col.append(&progress);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&col));
    let dialog = adw::Dialog::builder()
        .title("Add Printer")
        .content_width(460)
        .child(&toolbar)
        .default_widget(add.widget())
        .focus_widget(&name)
        .build();

    let cancel_token = CancelToken::new();
    cancel.connect_clicked({
        let dialog = dialog.clone();
        let token = cancel_token.clone();
        move |_| {
            token.cancel();
            dialog.close();
        }
    });

    let submit = {
        let ctx = ctx.clone();
        let win = Rc::downgrade(win);
        let dialog = dialog.clone();
        let (name, info, location) = (name.clone(), info.clone(), location.clone());
        let (error, progress, add) = (error.clone(), progress.clone(), add.clone());
        move || {
            let queue = name.text().trim().to_string();
            if !is_valid_queue_name(&queue) {
                error.set_label("Queue names cannot contain spaces, \"/\" or \"#\" and must be shorter than 128 characters.");
                error.set_visible(true);
                return;
            }
            let exists = ctx.registry.list.borrow().devices.iter().any(|d| {
                d.queue
                    .as_ref()
                    .is_some_and(|q| q.name.eq_ignore_ascii_case(&queue))
            });
            if exists {
                error.set_label(&format!("A print queue named {queue} already exists."));
                error.set_visible(true);
                return;
            }
            error.set_visible(false);
            progress.set_visible(true);
            for e in [&name, &info, &location] {
                e.set_sensitive(false);
            }
            let spec = NewQueue {
                name: queue.clone(),
                device_uri: device_uri.clone(),
                info: info.text().trim().to_string(),
                location: location.text().trim().to_string(),
            };
            let cups = ctx.cups.clone();
            let token = cancel_token.clone();
            let ctx2 = ctx.clone();
            let win = win.clone();
            let dialog = dialog.clone();
            let (name, info, location) = (name.clone(), info.clone(), location.clone());
            let (error, progress) = (error.clone(), progress.clone());
            actions::run(
                &ctx,
                Some(&add),
                async move { cups.add_queue(&spec, &token).await },
                move |r| {
                    progress.set_visible(false);
                    for e in [&name, &info, &location] {
                        e.set_sensitive(true);
                    }
                    match r {
                        Ok(()) => {
                            ctx2.toast(&format!("Added {queue}. Apps can print to it now."));
                            ctx2.registry.refresh();
                            if let Some(w) = win.upgrade() {
                                w.select_when_listed(DeviceId::Queue(queue.clone()));
                            }
                            dialog.close();
                        }
                        Err(e) => {
                            error.set_label(&format!(
                                "The printer was not added. {}",
                                text::cups_error(&e)
                            ));
                            error.set_visible(true);
                        }
                    }
                },
            );
        }
    };
    let submit = Rc::new(submit);
    add.connect_clicked({
        let submit = submit.clone();
        move || submit()
    });
    for e in [&name, &info, &location] {
        let submit = submit.clone();
        e.connect_entry_activated(move |_| submit());
    }
    dialog.present(Some(&ctx.window));
}
