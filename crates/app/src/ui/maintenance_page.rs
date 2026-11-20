// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Maintenance tab: HPLIP cleaning, alignment and diagnostics. Shown only when
//! HPLIP is installed outside a sandbox (see spool_services::hplip).

use super::actions;
use super::text;
use super::widgets::BusyButton;
use crate::app::context::AppContext;
use adw::prelude::*;
use spool_services::hplip::{self, GuiTool, HplipError, HplipTools};
use spool_services::model::Device;
use spool_services::net::CancelToken;
use std::cell::RefCell;
use std::rc::Rc;

pub struct MaintenancePage {
    ctx: Rc<AppContext>,
    tools: HplipTools,
    root: gtk::ScrolledWindow,
    head_note: adw::ActionRow,
    clean_row: adw::ActionRow,
    align_row: adw::ActionRow,
    clean: BusyButton,
    align: BusyButton,
    diagnose: BusyButton,
    device: RefCell<Device>,
}

impl MaintenancePage {
    pub fn new(ctx: Rc<AppContext>, tools: HplipTools, device: &Device) -> Rc<Self> {
        let clean = BusyButton::new("_Clean…");
        clean.widget().set_valign(gtk::Align::Center);
        let clean_row = adw::ActionRow::builder()
            .title("Clean print head")
            .subtitle("Opens the HPLIP cleaning tool, which prints a check page after each level.")
            .build();
        clean_row.add_suffix(clean.widget());

        let align = BusyButton::new("_Align…");
        align.widget().set_valign(gtk::Align::Center);
        let align_row = adw::ActionRow::builder()
            .title("Align print head")
            .subtitle("Opens the HPLIP alignment tool and prints an alignment page.")
            .build();
        align_row.add_suffix(align.widget());

        let head_note = adw::ActionRow::builder()
            .title("Cleaning and alignment need an HPLIP print queue")
            .subtitle("HPLIP only works with printers set up through its own driver. Run hp-setup to add one.")
            .build();

        let head = adw::PreferencesGroup::builder().title("Print Head").build();
        head.add(&clean_row);
        head.add(&align_row);
        head.add(&head_note);

        let diagnose = BusyButton::new("_Run");
        diagnose.widget().set_valign(gtk::Align::Center);
        let diag_row = adw::ActionRow::builder()
            .title("Device report")
            .subtitle("Collects device details and status with hp-info.")
            .build();
        diag_row.add_suffix(diagnose.widget());
        let diag = adw::PreferencesGroup::builder()
            .title("Diagnostics")
            .build();
        diag.add(&diag_row);

        let col = gtk::Box::new(gtk::Orientation::Vertical, 24);
        col.set_margin_top(24);
        col.set_margin_bottom(24);
        col.set_margin_start(12);
        col.set_margin_end(12);
        col.append(&head);
        col.append(&diag);
        let clamp = adw::Clamp::builder().maximum_size(720).child(&col).build();
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .vexpand(true)
            .build();

        let p = Rc::new(Self {
            ctx,
            tools,
            root,
            head_note,
            clean_row,
            align_row,
            clean,
            align,
            diagnose,
            device: RefCell::new(device.clone()),
        });
        let weak = Rc::downgrade(&p);
        p.clean.connect_clicked(move || {
            if let Some(p) = weak.upgrade() {
                p.run_gui(GuiTool::Clean);
            }
        });
        let weak = Rc::downgrade(&p);
        p.align.connect_clicked(move || {
            if let Some(p) = weak.upgrade() {
                p.run_gui(GuiTool::Align);
            }
        });
        let weak = Rc::downgrade(&p);
        p.diagnose.connect_clicked(move || {
            if let Some(p) = weak.upgrade() {
                p.run_diagnostics();
            }
        });
        p.set_device(device);
        p
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.root
    }

    pub fn set_device(&self, device: &Device) {
        *self.device.borrow_mut() = device.clone();
        let hp_queue = self.hp_queue_uri().is_some();
        self.clean_row
            .set_visible(hp_queue && self.tools.clean.is_some());
        self.align_row
            .set_visible(hp_queue && self.tools.align.is_some());
        self.head_note.set_visible(!hp_queue);
        self.diagnose
            .widget()
            .set_sensitive(self.tools.info.is_some() && (hp_queue || device.host.is_some()));
    }

    /// hp: device URI of a CUPS queue on the HPLIP backend, if this is one.
    fn hp_queue_uri(&self) -> Option<String> {
        let d = self.device.borrow();
        let q = d.queue.as_ref()?;
        q.is_hplip().then(|| q.device_uri.clone()).flatten()
    }

    fn run_gui(self: &Rc<Self>, tool: GuiTool) {
        let Some(uri) = self.hp_queue_uri() else {
            return;
        };
        let button = match tool {
            GuiTool::Clean => &self.clean,
            GuiTool::Align => &self.align,
        };
        let tools = self.tools.clone();
        let weak = Rc::downgrade(self);
        let what = match tool {
            GuiTool::Clean => "Cleaning",
            GuiTool::Align => "Alignment",
        };
        self.ctx
            .toast(&format!("{what} tool opened in a separate window"));
        actions::run(
            &self.ctx,
            Some(button),
            async move { hplip::run_gui(&tools, tool, &uri, &CancelToken::new()).await },
            move |r| {
                let Some(p) = weak.upgrade() else { return };
                match r {
                    Ok(()) => p.ctx.toast(&format!("{what} tool closed")),
                    Err(e) => p.ctx.toast(&format!("{what} tool failed: {e}")),
                }
            },
        );
    }

    fn run_diagnostics(self: &Rc<Self>) {
        let tools = self.tools.clone();
        let queue_uri = self.hp_queue_uri();
        let host = self.device.borrow().host.clone();
        let weak = Rc::downgrade(self);
        actions::run(
            &self.ctx,
            Some(&self.diagnose),
            async move {
                let cancel = CancelToken::new();
                let uri = match (queue_uri, host) {
                    (Some(u), _) => u,
                    (None, Some(h)) => hplip::make_uri(&tools, &h, &cancel).await?,
                    (None, None) => return Err(HplipError::NotHpDevice),
                };
                hplip::diagnostics(&tools, &uri, &cancel).await
            },
            move |r| {
                let Some(p) = weak.upgrade() else { return };
                match r {
                    Ok(report) => p.show_report(&report),
                    Err(HplipError::Failed { output, .. }) if !output.trim().is_empty() => {
                        p.show_report(&output);
                        p.ctx.toast("hp-info reported a problem; see the report");
                    }
                    Err(e) => p.ctx.toast(&text::sentence(&e)),
                }
            },
        );
    }

    fn show_report(&self, report: &str) {
        let buffer = gtk::TextBuffer::new(None);
        buffer.set_text(report.trim());
        let view = gtk::TextView::builder()
            .buffer(&buffer)
            .editable(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(12)
            .bottom_margin(12)
            .left_margin(12)
            .right_margin(12)
            .build();
        view.update_property(&[gtk::accessible::Property::Label("Device report")]);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .build();
        let copy = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text("Copy Report")
            .build();
        copy.update_property(&[gtk::accessible::Property::Label("Copy Report")]);
        let header = adw::HeaderBar::new();
        header.pack_start(&copy);
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&scrolled));
        let dialog = adw::Dialog::builder()
            .title("Device Report")
            .content_width(640)
            .content_height(520)
            .child(&toolbar)
            .build();
        let text = report.trim().to_string();
        let ctx = self.ctx.clone();
        copy.connect_clicked(move |b| {
            b.clipboard().set_text(&text);
            ctx.toast("Report copied");
        });
        dialog.present(Some(&self.ctx.window));
    }
}
