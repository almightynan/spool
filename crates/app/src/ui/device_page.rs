// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! The selected device: status, jobs, scanning and maintenance tabs.

use super::dashboard::Dashboard;
use super::jobs_page::JobsPage;
use super::maintenance_page::MaintenancePage;
use super::scan_page::ScanPage;
use super::window::MainWindow;
use crate::app::context::AppContext;
use adw::prelude::*;
use spool_protocols::uri::Uri;
use spool_services::hplip::HplipAvailability;
use spool_services::model::{Device, DeviceId};
use std::cell::RefCell;
use std::rc::Rc;

pub struct DevicePage {
    ctx: Rc<AppContext>,
    bin: adw::BreakpointBin,
    title: adw::WindowTitle,
    stack: adw::ViewStack,
    switcher_bar: adw::ViewSwitcherBar,
    web: gtk::Button,
    more: gtk::MenuButton,
    dashboard: Rc<Dashboard>,
    jobs: Rc<JobsPage>,
    scan: RefCell<Option<Rc<ScanPage>>>,
    maintenance: Option<Rc<MaintenancePage>>,
    status_tab: adw::ViewStackPage,
    jobs_tab: adw::ViewStackPage,
    scan_holder: adw::Bin,
    scan_tab: adw::ViewStackPage,
    maint_tab: Option<adw::ViewStackPage>,
    device: RefCell<Device>,
}

/// The printer's own web page: printer-more-info from IPP (an address that
/// works without mDNS name resolution), else the DNS-SD adminurl. CUPS's own
/// queue pages on localhost are not the printer's page and are skipped.
fn web_url(more_info: Option<&str>, admin_url: Option<&str>) -> Option<String> {
    let usable = |u: &&str| {
        Uri::parse(u).is_some_and(|p| {
            matches!(p.scheme.as_str(), "http" | "https")
                && !matches!(p.host.as_str(), "localhost" | "127.0.0.1" | "::1")
        })
    };
    more_info
        .filter(usable)
        .or(admin_url.filter(usable))
        .map(str::to_string)
}

impl DevicePage {
    pub fn new(ctx: Rc<AppContext>, win: &Rc<MainWindow>, d: &Device) -> Rc<Self> {
        let stack = adw::ViewStack::new();
        let dashboard = Dashboard::new(ctx.clone(), d);
        let jobs = JobsPage::new(ctx.clone(), win, d);
        let status_tab = stack.add_titled_with_icon(
            dashboard.widget(),
            Some("status"),
            "Status",
            "printer-symbolic",
        );
        let jobs_tab = stack.add_titled_with_icon(
            jobs.widget(),
            Some("jobs"),
            "Jobs",
            "document-print-symbolic",
        );
        let scan_holder = adw::Bin::new();
        let scan_tab =
            stack.add_titled_with_icon(&scan_holder, Some("scan"), "Scan", "scanner-symbolic");
        let (maintenance, maint_tab) = match &ctx.hplip {
            HplipAvailability::Available(tools) => {
                let m = MaintenancePage::new(ctx.clone(), tools.clone(), d);
                let tab = stack.add_titled_with_icon(
                    m.widget(),
                    Some("maintenance"),
                    "Maintenance",
                    "preferences-other-symbolic",
                );
                (Some(m), Some(tab))
            }
            _ => (None, None),
        };

        let title = adw::WindowTitle::new(&d.name, "");
        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        let header = adw::HeaderBar::builder().title_widget(&switcher).build();
        let refresh = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Refresh")
            .action_name("win.refresh")
            .build();
        refresh.update_property(&[gtk::accessible::Property::Label("Refresh")]);
        let web = gtk::Button::builder()
            .icon_name("web-browser-symbolic")
            .tooltip_text("Open Printer Web Page")
            .visible(false)
            .build();
        web.update_property(&[gtk::accessible::Property::Label("Open Printer Web Page")]);
        let menu = gtk::gio::Menu::new();
        menu.append(Some("_Forget This Address"), Some("device.forget"));
        let more = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&menu)
            .tooltip_text("More Options")
            .visible(false)
            .build();
        more.update_property(&[gtk::accessible::Property::Label("More Options")]);
        header.pack_end(&more);
        header.pack_end(&refresh);
        header.pack_end(&web);

        let switcher_bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&stack));
        toolbar.add_bottom_bar(&switcher_bar);

        let bin = adw::BreakpointBin::builder()
            .width_request(360)
            .height_request(200)
            .child(&toolbar)
            .build();
        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            550.0,
            adw::LengthUnit::Sp,
        ));
        bp.add_setter(&header, "title-widget", Some(&title.to_value()));
        bp.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
        bin.add_breakpoint(bp);

        let page = Rc::new(Self {
            ctx,
            bin,
            title,
            stack,
            switcher_bar,
            web,
            more,
            dashboard,
            jobs,
            scan: RefCell::new(None),
            maintenance,
            status_tab,
            jobs_tab,
            scan_holder,
            scan_tab,
            maint_tab,
            device: RefCell::new(d.clone()),
        });

        let weak = Rc::downgrade(&page);
        page.dashboard.connect_status(move || {
            if let Some(p) = weak.upgrade() {
                p.update_web();
            }
        });
        let weak = Rc::downgrade(&page);
        page.web.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.open_web();
            }
        });
        let weak = Rc::downgrade(&page);
        page.stack.connect_visible_child_name_notify(move |_| {
            if let Some(p) = weak.upgrade() {
                p.tab_changed();
            }
        });
        let actions = gtk::gio::SimpleActionGroup::new();
        let forget = gtk::gio::SimpleAction::new("forget", None);
        let weak = Rc::downgrade(&page);
        forget.connect_activate(move |_, _| {
            if let Some(p) = weak.upgrade() {
                p.forget();
            }
        });
        actions.add_action(&forget);
        page.bin.insert_action_group("device", Some(&actions));

        page.update(d);
        page.tab_changed();
        page
    }

    /// Removes a device added by address from the saved list.
    fn forget(&self) {
        let Some(host) = self.device.borrow().saved_address.clone() else {
            return;
        };
        let registry = self.ctx.registry.clone();
        let ctx = self.ctx.clone();
        let fut = self.ctx.worker.call({
            let host = host.clone();
            async move { registry.remove_manual(host).await }
        });
        gtk::glib::spawn_future_local(async move {
            match fut.await {
                Ok(Ok(())) => {
                    ctx.update_settings(|s| s.manual_hosts.retain(|h| h != &host));
                    ctx.toast(&format!("Forgot {host}"));
                }
                _ => ctx.toast("Could not forget the address because the device list stopped"),
            }
        });
    }

    pub fn widget(&self) -> &adw::BreakpointBin {
        &self.bin
    }

    pub fn id(&self) -> DeviceId {
        self.device.borrow().id.clone()
    }

    pub fn refresh(&self) {
        self.dashboard.refresh();
        self.jobs.refresh();
        if let Some(s) = self.scan.borrow().as_ref() {
            if self.stack.visible_child_name().as_deref() == Some("scan") {
                s.refresh();
            }
        }
    }

    fn tab_changed(&self) {
        let name = self.stack.visible_child_name();
        let name = name.as_deref();
        self.jobs.set_active(name == Some("jobs"));
        if let Some(s) = self.scan.borrow().as_ref() {
            s.set_active(name == Some("scan"));
        }
    }

    pub fn update(self: &Rc<Self>, d: &Device) {
        *self.device.borrow_mut() = d.clone();
        self.title.set_title(&d.name);
        self.title
            .set_subtitle(d.make_and_model.as_deref().unwrap_or_default());

        let printer = d.is_printer();
        self.status_tab.set_visible(printer);
        self.jobs_tab.set_visible(printer);
        if let Some(t) = &self.maint_tab {
            t.set_visible(printer);
        }
        if printer {
            self.dashboard.set_device(d);
            self.jobs.set_device(d);
            if let Some(m) = &self.maintenance {
                m.set_device(d);
            }
        }

        match &d.scanner {
            Some(s) => {
                let existing = self.scan.borrow().clone();
                match existing {
                    Some(page) => page.set_base_url(&s.base_url),
                    None => {
                        let page = ScanPage::new(self.ctx.clone(), &s.base_url);
                        self.scan_holder.set_child(Some(page.widget()));
                        *self.scan.borrow_mut() = Some(page);
                    }
                }
                self.scan_tab.set_visible(true);
            }
            None => {
                self.scan_tab.set_visible(false);
                self.scan_holder.set_child(None::<&gtk::Widget>);
                *self.scan.borrow_mut() = None;
            }
        }

        // One tab needs no switcher.
        let tabs = [printer, printer, d.scanner.is_some()]
            .iter()
            .filter(|v| **v)
            .count();
        self.switcher_bar.set_visible(tabs > 1);
        if !printer && self.stack.visible_child_name().as_deref() != Some("scan") {
            self.stack.set_visible_child_name("scan");
        }
        self.more.set_visible(d.saved_address.is_some());
        self.update_web();
        self.tab_changed();
    }

    fn update_web(&self) {
        let status = self.dashboard.last_status();
        // The printer's own make-and-model is more complete than DNS-SD "ty".
        if let Some(m) = status
            .as_ref()
            .and_then(|s| s.info.make_and_model.as_deref())
        {
            self.title.set_subtitle(m);
        }
        let url = web_url(
            status.as_ref().and_then(|s| s.info.more_info.as_deref()),
            self.device.borrow().admin_url.as_deref(),
        );
        self.web.set_visible(url.is_some());
        self.web.set_tooltip_text(Some(&match &url {
            Some(u) => format!("Open Printer Web Page ({u})"),
            None => "Open Printer Web Page".into(),
        }));
    }

    fn open_web(&self) {
        let status = self.dashboard.last_status();
        let Some(url) = web_url(
            status.as_ref().and_then(|s| s.info.more_info.as_deref()),
            self.device.borrow().admin_url.as_deref(),
        ) else {
            return;
        };
        let ctx = self.ctx.clone();
        let shown = url.clone();
        gtk::UriLauncher::new(&url).launch(
            Some(&self.ctx.window),
            gtk::gio::Cancellable::NONE,
            move |r| match r {
                Ok(()) => ctx.toast(&format!("Opened {shown} in your browser")),
                Err(e) => ctx.toast(&format!("Could not open the web page: {}", e.message())),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::web_url;

    #[test]
    fn prefers_printer_more_info_and_skips_cups() {
        assert_eq!(
            web_url(
                Some("http://192.168.1.147/#hId-pgAirPrint"),
                Some("http://HP.local./")
            )
            .as_deref(),
            Some("http://192.168.1.147/#hId-pgAirPrint")
        );
        assert_eq!(
            web_url(
                Some("http://localhost:631/printers/Q"),
                Some("http://HP.local./")
            )
            .as_deref(),
            Some("http://HP.local./")
        );
        assert_eq!(web_url(Some("ipp://x/"), None), None);
    }
}
