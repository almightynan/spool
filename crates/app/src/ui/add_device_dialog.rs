// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! "Add Printer by Address": probes an IP address, host name or ipp:// URI.

use super::actions;
use super::text;
use super::widgets::BusyButton;
use super::window::MainWindow;
use crate::app::context::AppContext;
use adw::prelude::*;
use spool_services::discovery::probe::parse_input;
use spool_services::discovery::ProbeError;
use spool_services::model::DeviceId;
use std::rc::Rc;

pub fn present(ctx: &Rc<AppContext>, win: &Rc<MainWindow>) {
    let entry = adw::EntryRow::builder()
        .title("IP address or host name")
        .input_purpose(gtk::InputPurpose::Url)
        .build();
    let error = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["error"])
        .visible(false)
        .build();
    error.set_accessible_role(gtk::AccessibleRole::Alert);
    let group = adw::PreferencesGroup::builder()
        .description("For example 192.168.1.20, printer.local or ipp://192.168.1.20/ipp/print. The printer and scanner on that device are both added.")
        .build();
    group.add(&entry);

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
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&col));
    let dialog = adw::Dialog::builder()
        .title("Add Printer by Address")
        .content_width(440)
        .child(&toolbar)
        .default_widget(add.widget())
        .focus_widget(&entry)
        .build();

    cancel.connect_clicked({
        let dialog = dialog.clone();
        move |_| {
            dialog.close();
        }
    });

    let submit = {
        let ctx = ctx.clone();
        let win = Rc::downgrade(win);
        let dialog = dialog.clone();
        let entry = entry.clone();
        let error = error.clone();
        let add = add.clone();
        move || {
            let input = entry.text().trim().to_string();
            if parse_input(&input).is_none() {
                error.set_label(
                    "Enter an IP address, a host name or an ipp:// address, without spaces.",
                );
                error.set_visible(true);
                return;
            }
            error.set_visible(false);
            entry.set_sensitive(false);
            let registry = ctx.registry.clone();
            let probe_input = input.clone();
            let ctx2 = ctx.clone();
            let win = win.clone();
            let dialog = dialog.clone();
            let entry = entry.clone();
            let error = error.clone();
            actions::run(
                &ctx,
                Some(&add),
                async move { registry.add_manual(probe_input).await },
                move |r| {
                    entry.set_sensitive(true);
                    match r {
                        Ok(Ok(dev)) => {
                            let input2 = input.clone();
                            ctx2.update_settings(move |s| {
                                if !s.manual_hosts.contains(&input2) {
                                    s.manual_hosts.push(input2);
                                }
                            });
                            let what = match (dev.printer.is_some(), dev.scanner.is_some()) {
                                (true, true) => "printer and scanner",
                                (true, false) => "printer",
                                _ => "scanner",
                            };
                            ctx2.toast(&format!("Found {} ({what}) at {}", dev.name, dev.host));
                            if let Some(w) = win.upgrade() {
                                // A device that matches a CUPS queue is listed under
                                // the queue's name.
                                let list = ctx2.registry.list.borrow().clone();
                                let id = list
                                    .devices
                                    .iter()
                                    .find(|d| {
                                        d.queue.is_some()
                                            && d.host.as_deref() == Some(dev.host.as_str())
                                    })
                                    .map(|d| d.id.clone())
                                    .unwrap_or(DeviceId::Network(dev.key.clone()));
                                w.select_when_listed(id);
                            }
                            dialog.close();
                        }
                        Ok(Err(e)) => {
                            let msg = match &e {
                                ProbeError::Unreachable(n) => format!(
                                    "Nothing answered at {input}: {n}. Check the address and that the device is turned on."
                                ),
                                ProbeError::NothingFound => format!(
                                    "{input} answered, but not as an IPP printer or eSCL scanner."
                                ),
                                other => text::sentence(other),
                            };
                            error.set_label(&msg);
                            error.set_visible(true);
                        }
                        Err(e) => {
                            error.set_label(&text::sentence(&e));
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
    entry.connect_entry_activated(move |_| submit());
    dialog.present(Some(&ctx.window));
}
