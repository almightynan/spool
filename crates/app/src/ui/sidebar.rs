// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! The printer list: CUPS queues first, then devices found on the network.

use super::text;
use adw::prelude::*;
use spool_services::model::{CupsState, Device, DeviceId, DeviceList, DiscoveryState};
use std::cell::RefCell;
use std::rc::Rc;

/// How long the empty list says "Searching" before saying nothing was found.
const SEARCH_GRACE_SECS: u32 = 6;

/// Widgets of one row that change when the device changes.
struct RowParts {
    row: gtk::ListBoxRow,
    image: gtk::Image,
    title: gtk::Label,
    subtitle: gtk::Label,
    star: gtk::Image,
}

impl RowParts {
    fn new() -> Self {
        let image = gtk::Image::new();
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let subtitle = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", "dim-label"])
            .build();
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
        labels.set_hexpand(true);
        labels.append(&title);
        labels.append(&subtitle);
        let star = gtk::Image::from_icon_name("starred-symbolic");
        star.set_tooltip_text(Some("Default printer"));
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        b.set_margin_top(6);
        b.set_margin_bottom(6);
        b.append(&image);
        b.append(&labels);
        b.append(&star);
        let row = gtk::ListBoxRow::builder().child(&b).build();
        Self {
            row,
            image,
            title,
            subtitle,
            star,
        }
    }

    fn set(&self, d: &Device) {
        self.image.set_icon_name(Some(if d.is_printer() {
            "printer-symbolic"
        } else {
            "scanner-symbolic"
        }));
        self.title.set_label(&d.name);
        let sub = text::device_subtitle(d);
        self.subtitle.set_label(&sub);
        self.star.set_visible(d.is_default);
        let mut accessible = format!("{}, {sub}", d.name);
        if d.is_default {
            accessible.push_str(", default printer");
        }
        self.row
            .update_property(&[gtk::accessible::Property::Label(&accessible)]);
    }
}

type SelectHandler = Box<dyn Fn(Option<DeviceId>)>;

pub struct Sidebar {
    pub page: adw::NavigationPage,
    list: gtk::ListBox,
    stack: gtk::Stack,
    empty: adw::StatusPage,
    cups_banner: adw::Banner,
    discovery_banner: adw::Banner,
    ids: RefCell<Vec<DeviceId>>,
    rows: RefCell<Vec<RowParts>>,
    /// Section headings by row index.
    sections: Rc<RefCell<Vec<Option<&'static str>>>>,
    searching: Rc<RefCell<bool>>,
    on_select: RefCell<Option<SelectHandler>>,
    updating: RefCell<bool>,
}

impl Sidebar {
    pub fn new() -> Rc<Self> {
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add Printer by Address")
            .action_name("win.add-device")
            .build();
        add.update_property(&[gtk::accessible::Property::Label("Add Printer by Address")]);

        let menu = gtk::gio::Menu::new();
        menu.append(Some("_Refresh"), Some("win.refresh"));
        menu.append(Some("_Preferences"), Some("app.preferences"));
        menu.append(Some("_About Spool"), Some("app.about"));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text("Main Menu")
            .build();
        menu_button.update_property(&[gtk::accessible::Property::Label("Main Menu")]);

        let header = adw::HeaderBar::new();
        header.pack_start(&add);
        header.pack_end(&menu_button);

        let cups_banner = adw::Banner::builder()
            .title("The print service is not responding. Added printers and jobs are not shown.")
            .button_label("Try Again")
            .action_name("win.refresh")
            .build();
        let discovery_banner = adw::Banner::new("Network search is off. Add printers by address.");
        cups_banner.set_visible(false);
        discovery_banner.set_visible(false);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Printers")]);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .vexpand(true)
            .build();

        let loading = adw::StatusPage::builder()
            .title("Looking for Printers")
            .build();
        loading.set_paintable(Some(&adw::SpinnerPaintable::new(Some(&loading))));
        loading.add_css_class("compact");

        let add_button = gtk::Button::builder()
            .label("_Add by Address")
            .use_underline(true)
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .action_name("win.add-device")
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("printer-symbolic")
            .child(&add_button)
            .css_classes(["compact"])
            .build();

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&loading, Some("loading"));
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scrolled, Some("list"));

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&cups_banner);
        content.append(&discovery_banner);
        content.append(&stack);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&content));

        let page = adw::NavigationPage::builder()
            .title("Printers")
            .tag("sidebar")
            .child(&toolbar)
            .build();

        let sections: Rc<RefCell<Vec<Option<&'static str>>>> = Rc::new(RefCell::new(Vec::new()));
        list.set_header_func({
            let sections = sections.clone();
            move |row, _before| {
                let title = sections
                    .borrow()
                    .get(row.index() as usize)
                    .copied()
                    .flatten();
                match title {
                    Some(t) => {
                        let l = gtk::Label::builder()
                            .label(t)
                            .xalign(0.0)
                            .css_classes(["heading", "dim-label"])
                            .margin_top(12)
                            .margin_bottom(6)
                            .margin_start(12)
                            .build();
                        row.set_header(Some(&l));
                    }
                    None => row.set_header(None::<&gtk::Widget>),
                }
            }
        });

        let s = Rc::new(Self {
            page,
            list,
            stack,
            empty,
            cups_banner,
            discovery_banner,
            ids: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            sections,
            searching: Rc::new(RefCell::new(true)),
            on_select: RefCell::new(None),
            updating: RefCell::new(false),
        });
        s.show_empty_text();
        let weak = Rc::downgrade(&s);
        s.list.connect_row_selected(move |_, row| {
            let Some(s) = weak.upgrade() else { return };
            if *s.updating.borrow() {
                return;
            }
            let id = row.and_then(|r| s.ids.borrow().get(r.index() as usize).cloned());
            if let Some(f) = s.on_select.borrow().as_ref() {
                f(id);
            };
        });
        // Activating an already selected row (Enter, or a click in the
        // collapsed layout) opens it again.
        let weak = Rc::downgrade(&s);
        s.list.connect_row_activated(move |_, row| {
            let Some(s) = weak.upgrade() else { return };
            let id = s.ids.borrow().get(row.index() as usize).cloned();
            if let Some(f) = s.on_select.borrow().as_ref() {
                f(id);
            };
        });
        let weak = Rc::downgrade(&s);
        gtk::glib::timeout_add_seconds_local_once(SEARCH_GRACE_SECS, move || {
            if let Some(s) = weak.upgrade() {
                *s.searching.borrow_mut() = false;
                s.show_empty_text();
            }
        });
        s
    }

    fn show_empty_text(&self) {
        if *self.searching.borrow() {
            self.empty.set_title("Searching the Network");
            self.empty
                .set_description(Some("Printers and scanners appear here as they are found."));
        } else {
            self.empty.set_title("No Printers Found");
            self.empty.set_description(Some(
                "Turn the printer on and connect it to the same network as this computer, or add it by its address.",
            ));
        }
    }

    pub fn connect_selected(&self, f: impl Fn(Option<DeviceId>) + 'static) {
        *self.on_select.borrow_mut() = Some(Box::new(f));
    }

    pub fn selected(&self) -> Option<DeviceId> {
        let row = self.list.selected_row()?;
        self.ids.borrow().get(row.index() as usize).cloned()
    }

    pub fn select(&self, id: &DeviceId) {
        let idx = self.ids.borrow().iter().position(|i| i == id);
        if let Some(row) = idx.and_then(|i| self.list.row_at_index(i as i32)) {
            self.list.select_row(Some(&row));
        }
    }

    fn rebuild(&self, list: &DeviceList, ids: Vec<DeviceId>) {
        let selected = self.selected();
        *self.updating.borrow_mut() = true;
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let mut sections = Vec::new();
        let mut rows = Vec::new();
        let mut last_section = None;
        for d in &list.devices {
            let section = if d.queue.is_some() {
                "Added to This Computer"
            } else {
                "On the Network"
            };
            sections.push((last_section != Some(section)).then_some(section));
            last_section = Some(section);
            let parts = RowParts::new();
            parts.set(d);
            self.list.append(&parts.row);
            rows.push(parts);
        }
        *self.sections.borrow_mut() = sections;
        *self.rows.borrow_mut() = rows;
        *self.ids.borrow_mut() = ids;
        self.list.invalidate_headers();
        if let Some(id) = selected {
            self.select(&id);
        }
        *self.updating.borrow_mut() = false;
    }

    pub fn update(&self, list: &DeviceList) {
        // Hidden banners stay in the accessibility tree unless made invisible.
        let cups_down = matches!(list.cups, CupsState::Unavailable(_));
        self.cups_banner.set_revealed(cups_down);
        self.cups_banner.set_visible(cups_down);
        let mdns_off = matches!(list.discovery, DiscoveryState::Unavailable(_));
        self.discovery_banner.set_revealed(mdns_off);
        self.discovery_banner.set_visible(mdns_off);

        let new_ids: Vec<DeviceId> = list.devices.iter().map(|d| d.id.clone()).collect();
        if *self.ids.borrow() == new_ids {
            // Same rows: update them in place so focus and selection stay.
            for (parts, d) in self.rows.borrow().iter().zip(&list.devices) {
                parts.set(d);
            }
        } else {
            self.rebuild(list, new_ids);
        }

        let page = if !list.devices.is_empty() {
            "list"
        } else if list.cups == CupsState::Loading {
            "loading"
        } else {
            "empty"
        };
        self.stack.set_visible_child_name(page);
    }
}
