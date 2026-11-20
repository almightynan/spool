// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Status tab: printer state, notices, supplies, paper and details.

use super::text;
use super::widgets::StateView;
use crate::app::context::AppContext;
use adw::prelude::*;
use spool_protocols::printer::{
    Level, Marker, MarkerKind, PrinterStatus, Rgb, Severity, SupplyWarning, Tray, TrayLevel,
};
use spool_services::hplip::HplipAvailability;
use spool_services::model::Device;
use spool_services::monitor::{spawn_monitor, MonitorHandle, StatusSource};
use spool_services::poll::Polled;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

/// A titled boxed list that can be cleared and refilled.
struct Section {
    group: adw::PreferencesGroup,
    list: gtk::ListBox,
}

impl Section {
    fn new(title: &str) -> Self {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let group = adw::PreferencesGroup::builder().title(title).build();
        group.add(&list);
        Self { group, list }
    }

    fn set_rows(&self, rows: Vec<gtk::Widget>) {
        self.list.remove_all();
        for r in &rows {
            self.list.append(r);
        }
        self.group.set_visible(!rows.is_empty());
    }
}

fn row(title: &str, subtitle: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(gtk::glib::markup_escape_text(title).as_str())
        .subtitle(gtk::glib::markup_escape_text(subtitle).as_str())
        .subtitle_selectable(true)
        .build()
}

fn severity_icon(s: Severity) -> &'static str {
    match s {
        Severity::Error => "dialog-error-symbolic",
        Severity::Warning => "dialog-warning-symbolic",
        Severity::Report => "dialog-information-symbolic",
    }
}

fn capitalize(s: &str) -> String {
    text::sentence(s).trim_end_matches('.').to_string()
}

/// Round color dots for a marker; marker colors are data from the printer.
fn swatch(colors: &[Rgb]) -> gtk::DrawingArea {
    let colors: Vec<Rgb> = colors.to_vec();
    let n = colors.len().max(1) as i32;
    let area = gtk::DrawingArea::builder()
        .content_width(14 * n + 4 * (n - 1))
        .content_height(14)
        .valign(gtk::Align::Center)
        .build();
    area.set_draw_func(move |area, cr, _w, h| {
        let fg = area.color();
        for (i, c) in colors.iter().enumerate() {
            let x = i as f64 * 18.0 + 7.0;
            let y = h as f64 / 2.0;
            cr.arc(x, y, 6.5, 0.0, std::f64::consts::TAU);
            cr.set_source_rgb(c.0 as f64 / 255.0, c.1 as f64 / 255.0, c.2 as f64 / 255.0);
            let _ = cr.fill_preserve();
            // Outline so white and black dots show on any theme.
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.35);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        }
    });
    area.set_accessible_role(gtk::AccessibleRole::Presentation);
    area
}

fn level_text(m: &Marker) -> String {
    let base = match m.level {
        Level::Percent(p) => format!("{p}%"),
        Level::Unavailable => "Level not reported".into(),
        Level::Unknown => "Level unknown".into(),
        Level::SomeRemaining => "Some left, exact level unknown".into(),
    };
    match m.warning {
        Some(SupplyWarning::Low) => format!("{base}, low"),
        Some(SupplyWarning::Empty) => "Empty".into(),
        Some(SupplyWarning::AlmostFull) => format!("{base}, almost full"),
        Some(SupplyWarning::Full) => "Full".into(),
        None => base,
    }
}

fn marker_row(m: &Marker) -> gtk::Widget {
    let r = row(&capitalize(&m.name), &level_text(m));
    if !m.colors.is_empty() {
        r.add_prefix(&swatch(&m.colors));
    }
    if let Level::Percent(p) = m.level {
        let bar = gtk::LevelBar::builder()
            .min_value(0.0)
            .max_value(100.0)
            .value(p as f64)
            .width_request(140)
            .valign(gtk::Align::Center)
            .build();
        // Replace the default fractional offsets with the printer's own
        // threshold, so the bar turns to the warning color at the same point
        // the printer reports "low".
        for o in ["low", "high", "full"] {
            bar.remove_offset_value(Some(o));
        }
        if m.kind != MarkerKind::Waste {
            if let Some(low) = m.low_level {
                bar.add_offset_value("low", low as f64);
            }
            bar.add_offset_value("high", 100.0);
        }
        bar.update_property(&[gtk::accessible::Property::Label(&format!(
            "{} level",
            m.name
        ))]);
        r.add_suffix(&bar);
    }
    r.upcast()
}

fn tray_row(t: &Tray) -> gtk::Widget {
    let mut parts = Vec::new();
    if let Some(m) = &t.media {
        parts.push(m.clone());
    }
    if let Some(m) = &t.media_type {
        parts.push(m.clone());
    }
    parts.push(match t.level {
        TrayLevel::Sheets {
            current,
            max: Some(max),
        } => format!("About {current} of {max} sheets"),
        TrayLevel::Sheets { current, max: None } => format!("About {current} sheets"),
        TrayLevel::Empty => "Empty".into(),
        TrayLevel::NotEmpty => "Has paper".into(),
        TrayLevel::Unknown => "Paper level unknown".into(),
    });
    if t.status.offline {
        parts.push("offline".into());
    } else if t.status.critical_alert || t.status.broken {
        parts.push("needs attention".into());
    }
    row(&t.name, &parts.join(", ")).upcast()
}

pub struct Dashboard {
    ctx: Rc<AppContext>,
    view: StateView,
    banner: adw::Banner,
    status: Section,
    supplies: Section,
    paper: Section,
    details: Section,
    maintenance: Section,
    monitor: RefCell<Option<MonitorHandle>>,
    source: RefCell<Option<StatusSource>>,
    device: RefCell<Device>,
    last: RefCell<Option<(Arc<PrinterStatus>, SystemTime)>>,
    on_status: RefCell<Option<Box<dyn Fn()>>>,
}

impl Dashboard {
    pub fn new(ctx: Rc<AppContext>, device: &Device) -> Rc<Self> {
        let banner = adw::Banner::builder()
            .button_label("Try Again")
            .visible(false)
            .build();
        let status = Section::new("Status");
        let supplies = Section::new("Supplies");
        let paper = Section::new("Paper");
        let details = Section::new("Details");
        let maintenance = Section::new("Maintenance");

        let col = gtk::Box::new(gtk::Orientation::Vertical, 24);
        col.set_margin_top(24);
        col.set_margin_bottom(24);
        col.set_margin_start(12);
        col.set_margin_end(12);
        for g in [&status, &supplies, &paper, &details, &maintenance] {
            col.append(&g.group);
        }
        let clamp = adw::Clamp::builder().maximum_size(720).child(&col).build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .vexpand(true)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&banner);
        content.append(&scrolled);
        let view = StateView::new(&content);

        let d = Rc::new(Self {
            ctx,
            view,
            banner,
            status,
            supplies,
            paper,
            details,
            maintenance,
            monitor: RefCell::new(None),
            source: RefCell::new(None),
            device: RefCell::new(device.clone()),
            last: RefCell::new(None),
            on_status: RefCell::new(None),
        });
        let weak = Rc::downgrade(&d);
        d.view.set_retry(move || {
            if let Some(d) = weak.upgrade() {
                d.refresh();
            }
        });
        let weak = Rc::downgrade(&d);
        d.banner.connect_button_clicked(move |_| {
            if let Some(d) = weak.upgrade() {
                d.refresh();
            }
        });
        d.fill_maintenance_note();
        d.set_device(device);
        d
    }

    pub fn widget(&self) -> &gtk::Stack {
        self.view.widget()
    }

    /// Called after each new status, so the page can update the web button.
    pub fn connect_status(&self, f: impl Fn() + 'static) {
        *self.on_status.borrow_mut() = Some(Box::new(f));
    }

    pub fn last_status(&self) -> Option<Arc<PrinterStatus>> {
        self.last.borrow().as_ref().map(|(s, _)| s.clone())
    }

    pub fn refresh(&self) {
        if let Some(m) = self.monitor.borrow().as_ref() {
            m.refresh();
        }
    }

    pub fn set_device(self: &Rc<Self>, device: &Device) {
        *self.device.borrow_mut() = device.clone();
        let source = device.status_source();
        if *self.source.borrow() == source {
            if let Some(s) = self.last_status() {
                self.render(&s);
            }
            return;
        }
        *self.source.borrow_mut() = source.clone();
        *self.last.borrow_mut() = None;
        *self.monitor.borrow_mut() = None;
        let Some(source) = source else { return };
        self.view.loading("Reading Printer Status");
        let handle = spawn_monitor(
            &self.ctx.worker.handle(),
            source,
            self.ctx.cups.clone(),
            &self.ctx.poll,
        );
        let weak = Rc::downgrade(self);
        crate::runtime::follow(handle.state.clone(), move |st| {
            let Some(d) = weak.upgrade() else {
                return false;
            };
            d.on_state(st);
            true
        });
        *self.monitor.borrow_mut() = Some(handle);
    }

    fn on_state(&self, st: Polled<Arc<PrinterStatus>, spool_services::monitor::StatusError>) {
        match st {
            Polled::Loading => {}
            Polled::Ready(s) => {
                *self.last.borrow_mut() = Some((s.clone(), SystemTime::now()));
                self.banner.set_revealed(false);
                self.banner.set_visible(false);
                self.render(&s);
                self.view.content();
            }
            Polled::Failed(e) if e.is_unreachable() => {
                let host = self
                    .device
                    .borrow()
                    .host
                    .clone()
                    .unwrap_or_else(|| "the printer".into());
                let last = self.last.borrow().clone();
                match last {
                    Some((_, at)) => {
                        self.banner.set_title(&format!(
                            "{host} has not answered since {}. Showing the last known status.",
                            text::clock(at)
                        ));
                        self.banner.set_visible(true);
                        self.banner.set_revealed(true);
                        self.view.content();
                    }
                    None => {
                        let first = match e.net_error() {
                            Some(n) => text::no_answer(&host, n),
                            None => format!("{host} did not answer."),
                        };
                        self.view.offline(
                            "Printer Is Offline",
                            &format!("{first} Check that the printer is turned on and connected to the same network."),
                        )
                    }
                }
            }
            Polled::Failed(e) => {
                self.view
                    .error("Could Not Read Printer Status", &text::sentence(&e));
            }
        }
        if let Some(f) = self.on_status.borrow().as_ref() {
            f();
        }
    }

    fn fill_maintenance_note(&self) {
        let note = match &self.ctx.hplip {
            HplipAvailability::Sandboxed => Some(
                "Cleaning and alignment are not available in the Flatpak version because HPLIP tools run outside the sandbox.",
            ),
            HplipAvailability::NotInstalled => {
                Some("Install HPLIP to clean and align the print head from here.")
            }
            HplipAvailability::Available(_) => None,
        };
        match note {
            Some(n) => self
                .maintenance
                .set_rows(vec![row("Maintenance tools unavailable", n).upcast()]),
            None => self.maintenance.set_rows(vec![]),
        }
    }

    fn render(&self, s: &PrinterStatus) {
        let device = self.device.borrow().clone();

        let mut status_rows: Vec<gtk::Widget> = Vec::new();
        let state = row(
            text::printer_state(s.state),
            s.state_message
                .as_deref()
                .unwrap_or(match s.accepting_jobs {
                    Some(false) => "Not accepting new jobs",
                    _ => "",
                }),
        );
        state.add_prefix(&gtk::Image::from_icon_name("printer-symbolic"));
        status_rows.push(state.upcast());
        for n in s.notices() {
            let r = row(&n.text, "");
            r.add_prefix(&gtk::Image::from_icon_name(severity_icon(n.severity)));
            status_rows.push(r.upcast());
        }
        self.status.set_rows(status_rows);

        let title = match (
            s.markers.iter().all(|m| m.kind == MarkerKind::Ink),
            s.markers.iter().all(|m| m.kind == MarkerKind::Toner),
        ) {
            (true, _) => "Ink",
            (_, true) => "Toner",
            _ => "Supplies",
        };
        self.supplies.group.set_title(title);
        let supply_rows: Vec<gtk::Widget> = if s.markers.is_empty() {
            vec![row(
                "Levels not reported",
                "The printer does not report ink or toner levels.",
            )
            .upcast()]
        } else {
            s.markers.iter().map(marker_row).collect()
        };
        self.supplies.set_rows(supply_rows);

        let paper_rows: Vec<gtk::Widget> = if !s.trays.is_empty() {
            s.trays.iter().map(tray_row).collect()
        } else if !s.media_ready.is_empty() {
            s.media_ready
                .iter()
                .map(|m| {
                    let title = m
                        .source
                        .clone()
                        .map(|s| capitalize(&s))
                        .unwrap_or_else(|| "Loaded paper".into());
                    let sub = match &m.media_type {
                        Some(t) => format!("{}, {}", m.size, t),
                        None => m.size.clone(),
                    };
                    row(&title, &sub).upcast()
                })
                .collect()
        } else {
            vec![row(
                "Paper status not reported",
                "The printer does not report its trays or loaded paper.",
            )
            .upcast()]
        };
        self.paper.set_rows(paper_rows);

        let mut details: Vec<gtk::Widget> = Vec::new();
        let model = s
            .info
            .make_and_model
            .clone()
            .or(device.make_and_model.clone());
        if let Some(m) = model {
            details.push(row("Model", &m).upcast());
        }
        if let Some(l) = s.info.location.clone().or(device.location.clone()) {
            details.push(row("Location", &l).upcast());
        }
        if let Some(h) = &device.host {
            details.push(row("Network address", h).upcast());
        }
        if let Some(f) = &s.info.firmware {
            details.push(row("Firmware", f).upcast());
        }
        details.push(
            row(
                "Print queue",
                device
                    .queue
                    .as_ref()
                    .map(|q| q.name.as_str())
                    .unwrap_or("Not added to this computer"),
            )
            .upcast(),
        );
        if let Some((_, at)) = self.last.borrow().as_ref() {
            details.push(row("Last updated", &text::clock(*at)).upcast());
        }
        self.details.set_rows(details);
    }
}
