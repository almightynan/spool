// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Jobs tab: the CUPS queue for this printer and its jobs.

use super::actions;
use super::add_queue_dialog;
use super::text;
use super::widgets::{BusyButton, StateView};
use super::window::MainWindow;
use crate::app::context::AppContext;
use adw::prelude::*;
use spool_protocols::jobs::Job;
use spool_services::cups::{CupsError, JobOp, WhichJobs};
use spool_services::model::{CupsState, Device};
use spool_services::poll::{spawn_poller, Polled, PollerHandle};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// Completed jobs shown; CUPS keeps more, but older ones are rarely useful.
const COMPLETED_SHOWN: usize = 20;

type Jobs = (Vec<Job>, Vec<Job>);

pub struct JobsPage {
    ctx: Rc<AppContext>,
    win: Weak<MainWindow>,
    view: StateView,
    default_button: BusyButton,
    default_label: gtk::Label,
    test_button: BusyButton,
    remove_button: BusyButton,
    active: gtk::ListBox,
    completed: gtk::ListBox,
    poller: RefCell<Option<PollerHandle<Jobs, CupsError>>>,
    device: RefCell<Device>,
    active_tab: RefCell<bool>,
}

fn boxed_list() -> gtk::ListBox {
    gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build()
}

fn group(title: &str, child: &impl IsA<gtk::Widget>) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder().title(title).build();
    g.add(child);
    g
}

/// Row shown in an empty list. ListBox placeholders are not used because
/// `remove_all` removes them too.
fn empty_row(text: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(text)
        .css_classes(["dim-label"])
        .activatable(false)
        .build()
}

impl JobsPage {
    pub fn new(ctx: Rc<AppContext>, win: &Rc<MainWindow>, device: &Device) -> Rc<Self> {
        let default_button = BusyButton::new("Make _Default");
        default_button.widget().set_valign(gtk::Align::Center);
        let default_label = gtk::Label::builder()
            .label("This is the default printer")
            .css_classes(["dim-label"])
            .build();
        let default_row = adw::ActionRow::builder()
            .title("Default printer")
            .subtitle("Apps print here unless you pick another printer.")
            .build();
        default_row.add_suffix(default_button.widget());
        default_row.add_suffix(&default_label);

        let test_button = BusyButton::new("Print _Test Page");
        test_button.widget().set_valign(gtk::Align::Center);
        let test_row = adw::ActionRow::builder()
            .title("Test page")
            .subtitle("Prints the CUPS test page with color bars and printer details.")
            .build();
        test_row.add_suffix(test_button.widget());

        let remove_button = BusyButton::new("_Remove…");
        remove_button.widget().set_valign(gtk::Align::Center);
        remove_button.widget().add_css_class("destructive-action");
        let remove_row = adw::ActionRow::builder()
            .title("Remove printer")
            .subtitle("Deletes the print queue from this computer.")
            .build();
        remove_row.add_suffix(remove_button.widget());

        let queue_list = boxed_list();
        queue_list.append(&default_row);
        queue_list.append(&test_row);
        queue_list.append(&remove_row);

        let active = boxed_list();
        let completed = boxed_list();

        let col = gtk::Box::new(gtk::Orientation::Vertical, 24);
        col.set_margin_top(24);
        col.set_margin_bottom(24);
        col.set_margin_start(12);
        col.set_margin_end(12);
        col.append(&group("Print Queue", &queue_list));
        col.append(&group("Active Jobs", &active));
        col.append(&group("Finished Jobs", &completed));
        let clamp = adw::Clamp::builder().maximum_size(720).child(&col).build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .vexpand(true)
            .build();
        let view = StateView::new(&scrolled);

        let p = Rc::new(Self {
            ctx,
            win: Rc::downgrade(win),
            view,
            default_button,
            default_label,
            test_button,
            remove_button,
            active,
            completed,
            poller: RefCell::new(None),
            device: RefCell::new(device.clone()),
            active_tab: RefCell::new(false),
        });
        let weak = Rc::downgrade(&p);
        p.view.set_retry(move || {
            if let Some(p) = weak.upgrade() {
                p.ctx.registry.refresh();
                p.refresh();
            }
        });
        let weak = Rc::downgrade(&p);
        p.default_button.connect_clicked(move || {
            if let Some(p) = weak.upgrade() {
                p.make_default();
            }
        });
        let weak = Rc::downgrade(&p);
        p.test_button.connect_clicked(move || {
            if let Some(p) = weak.upgrade() {
                p.test_page();
            }
        });
        let weak = Rc::downgrade(&p);
        p.remove_button.connect_clicked(move || {
            if let Some(p) = weak.upgrade() {
                p.confirm_remove();
            }
        });
        p.set_device(device);
        p
    }

    pub fn widget(&self) -> &gtk::Stack {
        self.view.widget()
    }

    fn queue_name(&self) -> Option<String> {
        self.device.borrow().queue.as_ref().map(|q| q.name.clone())
    }

    pub fn refresh(&self) {
        if let Some(p) = self.poller.borrow().as_ref() {
            p.refresh();
        }
    }

    /// Jobs are polled only while this tab is on screen.
    pub fn set_active(self: &Rc<Self>, active: bool) {
        *self.active_tab.borrow_mut() = active;
        self.update_poller();
    }

    pub fn set_device(self: &Rc<Self>, device: &Device) {
        let queue_changed = device.queue.as_ref().map(|q| &q.name)
            != self.device.borrow().queue.as_ref().map(|q| &q.name);
        *self.device.borrow_mut() = device.clone();
        self.default_button.widget().set_visible(!device.is_default);
        self.default_label.set_visible(device.is_default);
        if queue_changed {
            *self.poller.borrow_mut() = None;
        }
        self.update_poller();
    }

    fn update_poller(self: &Rc<Self>) {
        let device = self.device.borrow().clone();
        let cups_down = self.ctx.registry.list.borrow().cups.clone();
        if let CupsState::Unavailable(e) = cups_down {
            *self.poller.borrow_mut() = None;
            self.view
                .error("Print Service Not Responding", &text::cups_error(&e));
            return;
        }
        let Some(name) = device.queue.as_ref().map(|q| q.name.clone()) else {
            *self.poller.borrow_mut() = None;
            let weak = Rc::downgrade(self);
            self.view.with_action(
                "printer-symbolic",
                "Not Added to This Computer",
                "Add this printer to print to it from any app and to see its jobs here.",
                "_Add Printer…",
                move || {
                    if let Some(p) = weak.upgrade() {
                        if let Some(w) = p.win.upgrade() {
                            add_queue_dialog::present(&p.ctx, &w, &p.device.borrow());
                        }
                    }
                },
            );
            return;
        };
        if !*self.active_tab.borrow() {
            *self.poller.borrow_mut() = None;
            return;
        }
        if self.poller.borrow().is_some() {
            return;
        }
        if !self.view.is_content() {
            self.view.loading("Reading Jobs");
        }
        let cups = self.ctx.cups.clone();
        let handle = spawn_poller(&self.ctx.worker.handle(), &self.ctx.poll, move |cancel| {
            let cups = cups.clone();
            let name = name.clone();
            async move {
                let active = cups.jobs(&name, WhichJobs::NotCompleted, &cancel).await?;
                let done = cups.jobs(&name, WhichJobs::Completed, &cancel).await?;
                Ok((active, done))
            }
        });
        let weak = Rc::downgrade(self);
        crate::runtime::follow(handle.state.clone(), move |st| {
            let Some(p) = weak.upgrade() else {
                return false;
            };
            match st {
                Polled::Loading => {}
                Polled::Ready((a, d)) => {
                    p.render(&a, &d);
                    p.view.content();
                }
                Polled::Failed(e) => p.view.error("Could Not Read Jobs", &text::cups_error(&e)),
            }
            true
        });
        *self.poller.borrow_mut() = Some(handle);
    }

    fn job_row(self: &Rc<Self>, job: &Job) -> adw::ActionRow {
        let mut parts = vec![job.state.label().to_string()];
        if let Some(p) = job.pages_completed {
            parts.push(if p == 1 {
                "1 page".into()
            } else {
                format!("{p} pages")
            });
        }
        if let Some(u) = &job.user {
            parts.push(u.clone());
        }
        if let Some(t) = job.completed.or(job.created) {
            parts.push(text::clock_unix(t));
        }
        let r = adw::ActionRow::builder()
            .title(gtk::glib::markup_escape_text(&job.name).as_str())
            .subtitle(gtk::glib::markup_escape_text(&parts.join(" · ")).as_str())
            .build();
        let a = job.actions();
        let add = |enabled: bool, icon: &str, tip: &str, op: JobOp| {
            if !enabled {
                return;
            }
            let b = BusyButton::icon(icon, tip);
            let weak = Rc::downgrade(self);
            let job = job.clone();
            let bb = b.clone();
            b.connect_clicked(move || {
                if let Some(p) = weak.upgrade() {
                    p.job_action(&bb, &job, op);
                }
            });
            r.add_suffix(b.widget());
        };
        add(
            a.hold,
            "media-playback-pause-symbolic",
            "Hold Job",
            JobOp::Hold,
        );
        add(
            a.release,
            "media-playback-start-symbolic",
            "Release Job",
            JobOp::Release,
        );
        add(
            a.retry,
            "view-refresh-symbolic",
            "Print Again",
            JobOp::Restart,
        );
        add(
            a.cancel,
            "process-stop-symbolic",
            "Cancel Job",
            JobOp::Cancel,
        );
        r
    }

    fn render(self: &Rc<Self>, active: &[Job], done: &[Job]) {
        self.active.remove_all();
        for j in active {
            self.active.append(&self.job_row(j));
        }
        if active.is_empty() {
            self.active
                .append(&empty_row("No jobs are waiting or printing"));
        }
        self.completed.remove_all();
        for j in done.iter().take(COMPLETED_SHOWN) {
            self.completed.append(&self.job_row(j));
        }
        if done.is_empty() {
            self.completed.append(&empty_row("No finished jobs"));
        }
    }

    fn job_action(self: &Rc<Self>, button: &BusyButton, job: &Job, op: JobOp) {
        let Some(queue) = self.queue_name() else {
            return;
        };
        let cups = self.ctx.cups.clone();
        let id = job.id;
        let name = job.name.clone();
        let weak = Rc::downgrade(self);
        actions::run(
            &self.ctx,
            Some(button),
            async move { cups.job_op(&queue, id, op, &Default::default()).await },
            move |r| {
                let Some(p) = weak.upgrade() else { return };
                match r {
                    Ok(()) => {
                        let verb = match op {
                            JobOp::Cancel => "Canceled",
                            JobOp::Hold => "Held",
                            JobOp::Release => "Released",
                            JobOp::Restart => "Sent again:",
                        };
                        p.ctx.toast(&format!("{verb} \"{name}\""));
                    }
                    Err(CupsError::NotPossible(_)) if op == JobOp::Restart => {
                        p.ctx.toast(&format!(
                        "\"{name}\" cannot be printed again because CUPS no longer keeps its files"
                    ))
                    }
                    Err(e) => p.ctx.toast(&text::cups_error(&e)),
                }
                p.refresh();
            },
        );
    }

    fn make_default(self: &Rc<Self>) {
        let Some(queue) = self.queue_name() else {
            return;
        };
        let cups = self.ctx.cups.clone();
        let weak = Rc::downgrade(self);
        let q = queue.clone();
        actions::run(
            &self.ctx,
            Some(&self.default_button),
            async move { cups.set_default(&q, &Default::default()).await },
            move |r| {
                let Some(p) = weak.upgrade() else { return };
                match r {
                    Ok(()) => p.ctx.toast(&format!("{queue} is now the default printer")),
                    Err(e) => p.ctx.toast(&text::cups_error(&e)),
                }
                p.ctx.registry.refresh();
            },
        );
    }

    fn test_page(self: &Rc<Self>) {
        let Some(queue) = self.queue_name() else {
            return;
        };
        let cups = self.ctx.cups.clone();
        let weak = Rc::downgrade(self);
        let q = queue.clone();
        actions::run(
            &self.ctx,
            Some(&self.test_button),
            async move { cups.print_test_page(&q, &Default::default()).await },
            move |r| {
                let Some(p) = weak.upgrade() else { return };
                match r {
                    Ok(id) => p
                        .ctx
                        .toast(&format!("Test page sent to {queue} as job {id}")),
                    Err(e) => p.ctx.toast(&text::cups_error(&e)),
                }
                p.refresh();
            },
        );
    }

    fn confirm_remove(self: &Rc<Self>) {
        let Some(queue) = self.queue_name() else {
            return;
        };
        let dialog = adw::AlertDialog::new(
            Some(&format!("Remove {queue}?")),
            Some("Jobs waiting in this queue are canceled. The printer stays on the network and can be added again."),
        );
        dialog.add_responses(&[("cancel", "_Cancel"), ("remove", "_Remove")]);
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "remove" {
                return;
            }
            let Some(p) = weak.upgrade() else { return };
            let cups = p.ctx.cups.clone();
            let q = queue.clone();
            let weak = Rc::downgrade(&p);
            let queue = queue.clone();
            actions::run(
                &p.ctx,
                Some(&p.remove_button),
                async move { cups.delete_queue(&q, &Default::default()).await },
                move |r| {
                    let Some(p) = weak.upgrade() else { return };
                    match r {
                        Ok(()) => p.ctx.toast(&format!("Removed {queue}")),
                        Err(e) => p.ctx.toast(&text::cups_error(&e)),
                    }
                    p.ctx.registry.refresh();
                },
            );
        });
        dialog.present(Some(&self.ctx.window));
    }
}
