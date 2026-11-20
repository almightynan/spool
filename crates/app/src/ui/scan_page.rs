// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Scan tab: eSCL options, preview, scan, cancel and save.

use super::actions;
use super::text;
use super::widgets::{BusyButton, StateView};
use crate::app::context::AppContext;
use adw::prelude::*;
use spool_protocols::escl::{
    ColorMode, InputCaps, InputSource, PaperSize, ScanRequest, ScanSettings, ScannerCapabilities,
};
use spool_services::net::CancelToken;
use spool_services::scan::{
    save_scan, SaveFormat, ScanClient, ScanError, ScanProgress, ScannedPage,
};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Preview,
    Final,
}

/// Why a scan did not produce pages.
#[derive(Debug, Clone)]
enum Failure {
    NotReady(String),
    Scan(ScanError),
}

struct Scanned {
    pages: Vec<ScannedPage>,
    first: Option<gtk::gdk::Texture>,
}

pub struct ScanPage {
    ctx: Rc<AppContext>,
    view: StateView,
    client: RefCell<ScanClient>,
    caps: RefCell<Option<ScannerCapabilities>>,
    picture: gtk::Picture,
    preview_stack: gtk::Stack,
    status: gtk::Label,
    source: adw::ComboRow,
    duplex: adw::SwitchRow,
    color: adw::ComboRow,
    resolution: adw::ComboRow,
    paper: adw::ComboRow,
    format: adw::ComboRow,
    preview_button: BusyButton,
    scan_button: BusyButton,
    cancel_button: gtk::Button,
    save_button: gtk::Button,
    // Option values behind each combo, in combo order.
    sources: RefCell<Vec<InputSource>>,
    colors: RefCell<Vec<ColorMode>>,
    resolutions: RefCell<Vec<u32>>,
    papers: RefCell<Vec<PaperSize>>,
    formats: RefCell<Vec<SaveFormat>>,
    cancel: RefCell<Option<CancelToken>>,
    result: RefCell<Option<Vec<ScannedPage>>>,
    loaded: RefCell<bool>,
}

fn combo(title: &str) -> adw::ComboRow {
    adw::ComboRow::builder().title(title).build()
}

fn set_items(c: &adw::ComboRow, items: &[String], keep: Option<u32>) {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    c.set_model(Some(&gtk::StringList::new(&refs)));
    if let Some(i) = keep.filter(|i| (*i as usize) < items.len()) {
        c.set_selected(i);
    }
    c.set_sensitive(items.len() > 1);
}

fn source_label(s: InputSource) -> &'static str {
    match s {
        InputSource::Platen => "Flatbed",
        InputSource::Feeder => "Document feeder",
    }
}

impl ScanPage {
    pub fn new(ctx: Rc<AppContext>, base_url: &str) -> Rc<Self> {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .build();
        picture.update_property(&[gtk::accessible::Property::Label("Scan preview")]);
        let empty = adw::StatusPage::builder()
            .icon_name("scanner-symbolic")
            .title("No Preview Yet")
            .description("Place the document on the scanner and choose Preview.")
            .css_classes(["compact"])
            .build();
        let preview_stack = gtk::Stack::builder()
            .height_request(320)
            .css_classes(["card"])
            .build();
        preview_stack.add_named(&empty, Some("empty"));
        preview_stack.add_named(&picture, Some("image"));

        let status = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .label(" ")
            .build();
        status.set_accessible_role(gtk::AccessibleRole::Status);

        let source = combo("Source");
        let duplex = adw::SwitchRow::builder().title("Both sides").build();
        let color = combo("Color");
        let resolution = combo("Resolution");
        let paper = combo("Paper size");
        let format = combo("File format");
        let options = adw::PreferencesGroup::builder()
            .title("Scan Settings")
            .build();
        for r in [&source, &color, &resolution, &paper, &format] {
            options.add(r);
        }
        options.add(&duplex);

        let preview_button = BusyButton::new("_Preview");
        let scan_button = BusyButton::new("_Scan");
        scan_button.widget().add_css_class("suggested-action");
        let cancel_button = gtk::Button::builder()
            .label("_Cancel Scan")
            .use_underline(true)
            .visible(false)
            .build();
        let save_button = gtk::Button::builder()
            .label("Sa_ve…")
            .use_underline(true)
            .sensitive(false)
            .build();
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        buttons.set_halign(gtk::Align::End);
        buttons.append(&cancel_button);
        buttons.append(preview_button.widget());
        buttons.append(scan_button.widget());
        buttons.append(&save_button);

        let col = gtk::Box::new(gtk::Orientation::Vertical, 18);
        col.set_margin_top(24);
        col.set_margin_bottom(24);
        col.set_margin_start(12);
        col.set_margin_end(12);
        col.append(&preview_stack);
        col.append(&status);
        col.append(&buttons);
        col.append(&options);
        let clamp = adw::Clamp::builder().maximum_size(720).child(&col).build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .vexpand(true)
            .build();
        let view = StateView::new(&scrolled);

        let p = Rc::new(Self {
            ctx,
            view,
            client: RefCell::new(ScanClient::new(base_url)),
            caps: RefCell::new(None),
            picture,
            preview_stack,
            status,
            source,
            duplex,
            color,
            resolution,
            paper,
            format,
            preview_button,
            scan_button,
            cancel_button,
            save_button,
            sources: RefCell::new(Vec::new()),
            colors: RefCell::new(Vec::new()),
            resolutions: RefCell::new(Vec::new()),
            papers: RefCell::new(Vec::new()),
            formats: RefCell::new(Vec::new()),
            cancel: RefCell::new(None),
            result: RefCell::new(None),
            loaded: RefCell::new(false),
        });
        let weak = Rc::downgrade(&p);
        p.view.set_retry(move || {
            if let Some(p) = weak.upgrade() {
                p.load();
            }
        });
        let weak = Rc::downgrade(&p);
        p.source.connect_selected_notify(move |_| {
            if let Some(p) = weak.upgrade() {
                p.fill_options();
            }
        });
        let weak = Rc::downgrade(&p);
        p.duplex.connect_active_notify(move |_| {
            if let Some(p) = weak.upgrade() {
                p.fill_options();
            }
        });
        let weak = Rc::downgrade(&p);
        p.preview_button.connect_clicked(move || {
            if let Some(p) = weak.upgrade() {
                p.start(Kind::Preview);
            }
        });
        let weak = Rc::downgrade(&p);
        p.scan_button.connect_clicked(move || {
            if let Some(p) = weak.upgrade() {
                p.start(Kind::Final);
            }
        });
        let weak = Rc::downgrade(&p);
        p.cancel_button.connect_clicked(move |b| {
            if let Some(p) = weak.upgrade() {
                if let Some(c) = p.cancel.borrow().as_ref() {
                    c.cancel();
                }
                b.set_sensitive(false);
                p.set_status("Canceling the scan", false);
            }
        });
        let weak = Rc::downgrade(&p);
        p.save_button.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.save();
            }
        });
        p
    }

    pub fn widget(&self) -> &gtk::Stack {
        self.view.widget()
    }

    /// The device's scanner URL changed (for example after DHCP renewal).
    pub fn set_base_url(&self, base_url: &str) {
        *self.client.borrow_mut() = ScanClient::new(base_url);
    }

    /// Capabilities are read the first time the tab is shown.
    pub fn set_active(self: &Rc<Self>, active: bool) {
        if active && !*self.loaded.borrow() {
            self.load();
        }
    }

    pub fn refresh(self: &Rc<Self>) {
        if self.cancel.borrow().is_none() {
            self.load();
        }
    }

    fn load(self: &Rc<Self>) {
        *self.loaded.borrow_mut() = true;
        self.view.loading("Reading Scanner Options");
        let client = self.client.borrow().clone();
        let weak = Rc::downgrade(self);
        actions::run(
            &self.ctx,
            None,
            async move { client.capabilities(&CancelToken::new()).await },
            move |r| {
                let Some(p) = weak.upgrade() else { return };
                match r {
                    Ok(caps) => {
                        if caps.sources().is_empty() {
                            p.view.empty(
                                "scanner-symbolic",
                                "No Scan Inputs",
                                "The scanner reports neither a flatbed nor a document feeder.",
                            );
                            return;
                        }
                        *p.caps.borrow_mut() = Some(caps);
                        p.fill_sources();
                        p.view.content();
                    }
                    Err(ScanError::Net(e)) if e.is_unreachable() => p.view.offline(
                        "Scanner Is Offline",
                        &format!(
                            "{} Check that it is turned on and connected to the same network.",
                            text::no_answer("The scanner", &e)
                        ),
                    ),
                    Err(e) => p
                        .view
                        .error("Could Not Read Scanner Options", &text::sentence(&e)),
                }
            },
        );
    }

    fn fill_sources(&self) {
        let Some(caps) = self.caps.borrow().clone() else {
            return;
        };
        let sources = caps.sources();
        let labels: Vec<String> = sources
            .iter()
            .map(|s| source_label(*s).to_string())
            .collect();
        *self.sources.borrow_mut() = sources;
        set_items(&self.source, &labels, Some(0));
        self.fill_options();
    }

    fn selected<T: Clone>(c: &adw::ComboRow, v: &RefCell<Vec<T>>) -> Option<T> {
        v.borrow().get(c.selected() as usize).cloned()
    }

    fn current_input(&self) -> Option<(InputSource, bool, InputCaps)> {
        let caps = self.caps.borrow();
        let caps = caps.as_ref()?;
        let source = Self::selected(&self.source, &self.sources)?;
        let duplex =
            source == InputSource::Feeder && caps.adf_duplex.is_some() && self.duplex.is_active();
        let input = caps.input(source, duplex)?.clone();
        Some((source, duplex, input))
    }

    /// Fills color, resolution, paper and format for the chosen source,
    /// keeping earlier choices where they still apply.
    fn fill_options(&self) {
        let Some(caps) = self.caps.borrow().clone() else {
            return;
        };
        let Some(source) = Self::selected(&self.source, &self.sources) else {
            return;
        };
        self.duplex
            .set_visible(source == InputSource::Feeder && caps.adf_duplex.is_some());
        let Some((_, _, input)) = self.current_input() else {
            return;
        };

        let prev_color = Self::selected(&self.color, &self.colors);
        let colors = input.color_modes.clone();
        let keep = prev_color
            .and_then(|c| colors.iter().position(|x| *x == c))
            .unwrap_or(0);
        set_items(
            &self.color,
            &colors.iter().map(|c| c.label()).collect::<Vec<_>>(),
            Some(keep as u32),
        );
        *self.colors.borrow_mut() = colors;

        let prev_res = Self::selected(&self.resolution, &self.resolutions);
        let res = input.resolutions.clone();
        // Default to 300 dpi, the usual document resolution, when offered.
        let keep = prev_res
            .and_then(|r| res.iter().position(|x| *x == r))
            .or_else(|| res.iter().position(|x| *x == 300))
            .unwrap_or(0);
        set_items(
            &self.resolution,
            &res.iter().map(|r| format!("{r} dpi")).collect::<Vec<_>>(),
            Some(keep as u32),
        );
        *self.resolutions.borrow_mut() = res;

        let prev_paper = Self::selected(&self.paper, &self.papers);
        let papers: Vec<PaperSize> = PaperSize::ALL
            .into_iter()
            .filter(|p| p.fits(&input))
            .collect();
        let keep = prev_paper
            .and_then(|p| papers.iter().position(|x| *x == p))
            .unwrap_or(0);
        set_items(
            &self.paper,
            &papers
                .iter()
                .map(|p| p.label().to_string())
                .collect::<Vec<_>>(),
            Some(keep as u32),
        );
        *self.papers.borrow_mut() = papers;

        let prev_fmt = Self::selected(&self.format, &self.formats);
        let mut formats = Vec::new();
        if input.supports_format("image/jpeg") || input.supports_format("application/pdf") {
            formats.push(SaveFormat::Pdf);
        }
        if input.supports_format("image/jpeg") {
            formats.push(SaveFormat::Jpeg);
        }
        let keep = prev_fmt
            .and_then(|f| formats.iter().position(|x| *x == f))
            .unwrap_or(0);
        let labels: Vec<String> = formats
            .iter()
            .map(|f| match f {
                SaveFormat::Pdf => "PDF document".to_string(),
                SaveFormat::Jpeg => "JPEG image".to_string(),
            })
            .collect();
        set_items(&self.format, &labels, Some(keep as u32));
        *self.formats.borrow_mut() = formats;

        // Preview needs an image the app can show.
        self.preview_button
            .widget()
            .set_visible(input.supports_format("image/jpeg"));
    }

    fn request(&self, kind: Kind) -> Option<ScanRequest> {
        let (source, duplex, input) = self.current_input()?;
        let color_mode = Self::selected(&self.color, &self.colors)?;
        let paper = Self::selected(&self.paper, &self.papers)?;
        let jpeg = input.supports_format("image/jpeg");
        let (resolution, intent) = match kind {
            // The lowest offered resolution of at least 75 dpi.
            Kind::Preview => (
                input
                    .resolutions
                    .iter()
                    .copied()
                    .find(|r| *r >= 75)
                    .or_else(|| input.resolutions.first().copied())?,
                input
                    .intents
                    .iter()
                    .any(|i| i == "Preview")
                    .then(|| "Preview".to_string()),
            ),
            Kind::Final => (Self::selected(&self.resolution, &self.resolutions)?, None),
        };
        Some(ScanRequest {
            source,
            // A preview only needs the first side.
            duplex: duplex && kind == Kind::Final,
            color_mode,
            resolution,
            paper,
            document_format: if jpeg {
                "image/jpeg"
            } else {
                "application/pdf"
            }
            .into(),
            intent,
        })
    }

    fn set_status(&self, msg: &str, error: bool) {
        self.status.set_label(msg);
        if error {
            self.status.remove_css_class("dim-label");
            self.status.add_css_class("error");
        } else {
            self.status.remove_css_class("error");
            self.status.add_css_class("dim-label");
        }
    }

    fn set_running(&self, running: Option<Kind>) {
        self.cancel_button.set_visible(running.is_some());
        self.cancel_button.set_sensitive(running.is_some());
        let idle = running.is_none();
        self.preview_button.set_busy(running == Some(Kind::Preview));
        self.scan_button.set_busy(running == Some(Kind::Final));
        if running == Some(Kind::Preview) {
            self.scan_button.widget().set_sensitive(false);
        }
        if running == Some(Kind::Final) {
            self.preview_button.widget().set_sensitive(false);
        }
        for r in [
            &self.source,
            &self.color,
            &self.resolution,
            &self.paper,
            &self.format,
        ] {
            r.set_sensitive(idle && r.model().is_some_and(|m| m.n_items() > 1));
        }
        self.duplex.set_sensitive(idle);
        self.save_button
            .set_sensitive(idle && self.result.borrow().is_some());
    }

    fn start(self: &Rc<Self>, kind: Kind) {
        if self.cancel.borrow().is_some() {
            return;
        }
        let (Some(caps), Some(req)) = (self.caps.borrow().clone(), self.request(kind)) else {
            self.set_status("Choose the scan settings first.", true);
            return;
        };
        let settings = match ScanSettings::for_request(&caps, &req) {
            Ok(s) => s,
            Err(e) => {
                self.set_status(&text::sentence(&e), true);
                return;
            }
        };
        let cancel = CancelToken::new();
        *self.cancel.borrow_mut() = Some(cancel.clone());
        self.set_running(Some(kind));
        // The pressed button turns insensitive; keep keyboard focus on the
        // control that matters now instead of letting GTK move it (and the
        // scroll position) somewhere else.
        self.cancel_button.grab_focus();
        self.set_status("Checking the scanner", false);

        let (tx, rx) = watch::channel(ScanProgress::Starting);
        let weak = Rc::downgrade(self);
        crate::runtime::follow(rx, move |p| {
            let Some(page) = weak.upgrade() else {
                return false;
            };
            if page
                .cancel
                .borrow()
                .as_ref()
                .is_some_and(|c| !c.is_cancelled())
            {
                let msg = match p {
                    ScanProgress::Starting => "Starting the scan".to_string(),
                    ScanProgress::Scanning { page: n } if n > 1 => format!("Scanning page {n}"),
                    ScanProgress::Scanning { .. } => "Scanning".to_string(),
                };
                page.set_status(&msg, false);
            }
            true
        });

        let client = self.client.borrow().clone();
        let feeder = req.source == InputSource::Feeder;
        let fut = async move {
            let status = client.status(&cancel).await.map_err(Failure::Scan)?;
            if !status.state.can_scan() {
                return Err(Failure::NotReady(format!(
                    "The scanner is not ready: {}.",
                    status.state.label().to_lowercase()
                )));
            }
            if feeder && status.adf_has_paper() == Some(false) {
                return Err(Failure::NotReady(
                    "The document feeder is empty. Load the pages and try again.".into(),
                ));
            }
            let pages = client
                .scan(&settings, &tx, &cancel)
                .await
                .map_err(Failure::Scan)?;
            // Decoding happens here, off the GTK thread.
            let first = pages.first().and_then(|p| {
                let bytes = gtk::glib::Bytes::from(&p.data);
                gtk::gdk::Texture::from_bytes(&bytes).ok()
            });
            Ok(Scanned { pages, first })
        };
        let weak = Rc::downgrade(self);
        let dpi = req.resolution;
        actions::run(&self.ctx, None, fut, move |r| {
            let Some(p) = weak.upgrade() else { return };
            *p.cancel.borrow_mut() = None;
            match r {
                Ok(s) => {
                    if let Some(t) = &s.first {
                        p.picture.set_paintable(Some(t));
                        p.preview_stack.set_visible_child_name("image");
                    }
                    let n = s.pages.len();
                    match kind {
                        Kind::Preview => p.set_status(
                            "Preview ready. Adjust the settings and choose Scan.",
                            false,
                        ),
                        Kind::Final => {
                            p.set_status(
                                &if n == 1 {
                                    format!("Scanned 1 page at {dpi} dpi. Choose Save to keep it.")
                                } else {
                                    format!(
                                        "Scanned {n} pages at {dpi} dpi. Choose Save to keep them."
                                    )
                                },
                                false,
                            );
                            *p.result.borrow_mut() = Some(s.pages);
                        }
                    }
                }
                Err(Failure::Scan(ScanError::Cancelled)) => {
                    p.set_status("Scan canceled.", false);
                    p.ctx.toast("Scan canceled");
                }
                Err(Failure::NotReady(m)) => p.set_status(&m, true),
                Err(Failure::Scan(e)) => p.set_status(&text::sentence(&e), true),
            }
            p.set_running(None);
        });
    }

    fn default_name(&self, format: SaveFormat) -> String {
        let stamp = gtk::glib::DateTime::now_local()
            .ok()
            .and_then(|d| d.format("%Y-%m-%d %H-%M-%S").ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "scan".into());
        format!("Scan {stamp}.{}", format.extension())
    }

    fn save(self: &Rc<Self>) {
        let Some(pages) = self.result.borrow().clone() else {
            return;
        };
        let Some(format) = Self::selected(&self.format, &self.formats) else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title("Save Scan")
            .modal(true)
            .initial_name(self.default_name(format))
            .build();
        if let Some(folder) = self.ctx.save_folder() {
            dialog.set_initial_folder(Some(&gtk::gio::File::for_path(folder)));
        }
        let weak = Rc::downgrade(self);
        dialog.save(
            Some(&self.ctx.window),
            gtk::gio::Cancellable::NONE,
            move |res| {
                let Some(p) = weak.upgrade() else { return };
                let Ok(file) = res else { return };
                let Some(path) = file.path() else {
                    p.ctx.toast("The chosen location is not a local folder");
                    return;
                };
                p.save_button.set_sensitive(false);
                let weak = Rc::downgrade(&p);
                let target = path.clone();
                actions::run(
                    &p.ctx,
                    None,
                    async move { save_scan(&pages, format, &target).await },
                    move |r| {
                        let Some(p) = weak.upgrade() else { return };
                        p.save_button.set_sensitive(true);
                        match r {
                            Ok(files) => p.saved(files),
                            Err(e) => p.ctx.toast(&format!("Could not save the scan: {e}")),
                        }
                    },
                );
            },
        );
    }

    fn saved(&self, files: Vec<PathBuf>) {
        let Some(first) = files.first().cloned() else {
            return;
        };
        let name = first
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let msg = if files.len() == 1 {
            format!("Saved {name}")
        } else {
            format!("Saved {} files, starting with {name}", files.len())
        };
        self.set_status(&format!("{msg}."), false);
        let window = self.ctx.window.clone();
        self.ctx.toast_with_action(&msg, "Show in Folder", move || {
            let launcher = gtk::FileLauncher::new(Some(&gtk::gio::File::for_path(&first)));
            launcher.open_containing_folder(Some(&window), gtk::gio::Cancellable::NONE, |_| {});
        });
    }
}
