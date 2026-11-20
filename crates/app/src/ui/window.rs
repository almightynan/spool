// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! The main window: printer list on the left, the selected device on the right.

use super::device_page::DevicePage;
use super::sidebar::Sidebar;
use crate::app::context::AppContext;
use adw::prelude::*;
use spool_services::model::{DeviceId, DeviceList};
use spool_services::poll::Visibility;
use std::cell::RefCell;
use std::rc::Rc;

pub struct MainWindow {
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    split: adw::NavigationSplitView,
    pub sidebar: Rc<Sidebar>,
    content_page: adw::NavigationPage,
    placeholder: adw::ToolbarView,
    device: RefCell<Option<Rc<DevicePage>>>,
    ctx: RefCell<Option<Rc<AppContext>>>,
    list: RefCell<Option<DeviceList>>,
    /// Device to select once it appears (after adding a queue or address).
    pending_select: RefCell<Option<DeviceId>>,
}

impl MainWindow {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let sidebar = Sidebar::new();

        let placeholder_status = adw::StatusPage::builder()
            .icon_name("printer-symbolic")
            .title("No Printer Selected")
            .description("Select a printer or scanner in the list.")
            .build();
        let placeholder = adw::ToolbarView::new();
        placeholder.add_top_bar(&adw::HeaderBar::new());
        placeholder.set_content(Some(&placeholder_status));
        let content_page = adw::NavigationPage::builder()
            .title("Printer")
            .tag("content")
            .child(&placeholder)
            .build();

        let split = adw::NavigationSplitView::builder()
            .sidebar(&sidebar.page)
            .content(&content_page)
            .min_sidebar_width(260.0)
            .build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Spool")
            .default_width(980)
            .default_height(700)
            .width_request(360)
            .height_request(294)
            .content(&toasts)
            .build();

        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            600.0,
            adw::LengthUnit::Sp,
        ));
        bp.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(bp);

        let w = Rc::new(Self {
            window,
            toasts,
            split,
            sidebar,
            content_page,
            placeholder,
            device: RefCell::new(None),
            ctx: RefCell::new(None),
            list: RefCell::new(None),
            pending_select: RefCell::new(None),
        });
        w.setup_actions();
        let weak = Rc::downgrade(&w);
        w.sidebar.connect_selected(move |id| {
            if let Some(w) = weak.upgrade() {
                w.show_device(id);
            }
        });
        w
    }

    fn setup_actions(self: &Rc<Self>) {
        let add = gtk::gio::SimpleAction::new("add-device", None);
        let weak = Rc::downgrade(self);
        add.connect_activate(move |_, _| {
            let Some(w) = weak.upgrade() else { return };
            if let Some(ctx) = w.context() {
                super::add_device_dialog::present(&ctx, &w);
            }
        });
        self.window.add_action(&add);

        let refresh = gtk::gio::SimpleAction::new("refresh", None);
        let weak = Rc::downgrade(self);
        refresh.connect_activate(move |_, _| {
            let Some(w) = weak.upgrade() else { return };
            if let Some(ctx) = w.ctx.borrow().as_ref() {
                ctx.registry.refresh();
            }
            if let Some(p) = w.device.borrow().as_ref() {
                p.refresh();
            };
        });
        self.window.add_action(&refresh);
    }

    /// Connects the window to the running app once start-up finished.
    pub fn attach(self: &Rc<Self>, ctx: Rc<AppContext>) {
        *self.ctx.borrow_mut() = Some(ctx.clone());

        let weak = Rc::downgrade(self);
        let poll = ctx.poll.clone();
        let update_visibility = move || {
            let Some(w) = weak.upgrade() else { return };
            let win = &w.window;
            let v = if !win.is_visible() || win.is_suspended() {
                Visibility::Hidden
            } else if win.is_active() {
                Visibility::Focused
            } else {
                Visibility::Unfocused
            };
            poll.set_visibility(v);
        };
        update_visibility();
        for prop in ["suspended", "is-active", "visible"] {
            let f = update_visibility.clone();
            self.window
                .connect_notify_local(Some(prop), move |_, _| f());
        }

        let weak = Rc::downgrade(self);
        crate::runtime::follow(ctx.registry.list.clone(), move |list| {
            let Some(w) = weak.upgrade() else {
                return false;
            };
            w.on_list(list);
            true
        });
    }

    pub fn context(&self) -> Option<Rc<AppContext>> {
        self.ctx.borrow().clone()
    }

    /// Selects `id` now if it is listed, or as soon as it appears.
    pub fn select_when_listed(self: &Rc<Self>, id: DeviceId) {
        let listed = self
            .list
            .borrow()
            .as_ref()
            .is_some_and(|l| l.get(&id).is_some());
        if listed {
            self.sidebar.select(&id);
            self.show_device(Some(id));
        } else {
            *self.pending_select.borrow_mut() = Some(id);
        }
    }

    fn on_list(self: &Rc<Self>, list: DeviceList) {
        self.sidebar.update(&list);
        let pending = self.pending_select.borrow().clone();
        *self.list.borrow_mut() = Some(list.clone());
        if let Some(id) = pending {
            if list.get(&id).is_some() {
                *self.pending_select.borrow_mut() = None;
                self.sidebar.select(&id);
                self.show_device(Some(id));
                return;
            }
        }
        let current = self.device.borrow().as_ref().map(|p| p.id());
        // In the wide layout, open the first device instead of an empty pane.
        if current.is_none() && !self.split.is_collapsed() {
            if let Some(first) = list.devices.first() {
                let id = first.id.clone();
                self.sidebar.select(&id);
                self.show_device(Some(id));
                return;
            }
        }
        match current.as_ref().and_then(|id| list.get(id)) {
            Some(d) => {
                let page = self.device.borrow().clone();
                if let Some(p) = page {
                    p.update(d);
                    self.content_page.set_title(&d.name);
                }
            }
            // The shown device left the list (queue removed, device gone).
            None if current.is_some() => self.show_device(None),
            None => {}
        }
    }

    fn show_device(self: &Rc<Self>, id: Option<DeviceId>) {
        let Some(ctx) = self.ctx.borrow().clone() else {
            return;
        };
        let device = id
            .as_ref()
            .and_then(|id| self.list.borrow().as_ref().and_then(|l| l.get(id).cloned()));
        match device {
            Some(d) => {
                let same = self
                    .device
                    .borrow()
                    .as_ref()
                    .is_some_and(|p| p.id() == d.id);
                if !same {
                    // Dropping the old page stops its polling.
                    let page = DevicePage::new(ctx, self, &d);
                    self.content_page.set_child(Some(page.widget()));
                    self.content_page.set_title(&d.name);
                    *self.device.borrow_mut() = Some(page);
                }
                self.split.set_show_content(true);
            }
            None => {
                *self.device.borrow_mut() = None;
                self.content_page.set_child(Some(&self.placeholder));
                self.content_page.set_title("Printer");
                self.split.set_show_content(false);
            }
        }
    }
}
