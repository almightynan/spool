// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Preferences: refresh interval, save folder, style.

use crate::app::context::AppContext;
use adw::prelude::*;
use spool_services::settings::{Style, REFRESH_MAX, REFRESH_MIN};
use std::rc::Rc;

pub fn present(ctx: &Rc<AppContext>) {
    let settings = ctx.settings.borrow().clone();

    let refresh = adw::SpinRow::with_range(REFRESH_MIN as f64, REFRESH_MAX as f64, 5.0);
    refresh.set_title("Refresh interval");
    refresh.set_subtitle("Seconds between status checks. Checks slow down when the window is in the background and stop while it is hidden.");
    refresh.set_value(settings.refresh_secs as f64);
    refresh.connect_value_notify({
        let ctx = ctx.clone();
        move |r| {
            let v = r
                .value()
                .round()
                .clamp(REFRESH_MIN as f64, REFRESH_MAX as f64) as u32;
            ctx.update_settings(|s| s.refresh_secs = v);
        }
    });
    let updates = adw::PreferencesGroup::builder()
        .title("Status Updates")
        .build();
    updates.add(&refresh);

    let folder_row = adw::ActionRow::builder()
        .title("Save folder")
        .subtitle_selectable(true)
        .build();
    let show_folder = {
        let ctx = ctx.clone();
        let row = folder_row.clone();
        move || {
            let text = ctx
                .save_folder()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "Not set".into());
            row.set_subtitle(&gtk::glib::markup_escape_text(&text));
        }
    };
    show_folder();
    let choose = gtk::Button::builder()
        .label("_Choose…")
        .use_underline(true)
        .valign(gtk::Align::Center)
        .build();
    folder_row.add_suffix(&choose);
    folder_row.set_activatable_widget(Some(&choose));
    choose.connect_clicked({
        let ctx = ctx.clone();
        let show_folder = show_folder.clone();
        move |b| {
            let dialog = gtk::FileDialog::builder()
                .title("Choose Save Folder")
                .modal(true)
                .build();
            if let Some(f) = ctx.save_folder() {
                dialog.set_initial_folder(Some(&gtk::gio::File::for_path(f)));
            }
            let ctx = ctx.clone();
            let show_folder = show_folder.clone();
            let parent = b.root().and_downcast::<gtk::Window>();
            dialog.select_folder(parent.as_ref(), gtk::gio::Cancellable::NONE, move |r| {
                let Ok(file) = r else { return };
                match file.path() {
                    Some(p) => {
                        ctx.update_settings(|s| s.save_folder = Some(p));
                        show_folder();
                    }
                    None => ctx.toast("The chosen folder is not a local folder"),
                }
            });
        }
    });
    let scanning = adw::PreferencesGroup::builder()
        .title("Scanning")
        .description("Scans are offered for saving in this folder.")
        .build();
    scanning.add(&folder_row);

    let styles = [Style::System, Style::Light, Style::Dark];
    let style = adw::ComboRow::builder()
        .title("Style")
        .model(&gtk::StringList::new(&["Follow system", "Light", "Dark"]))
        .selected(
            styles
                .iter()
                .position(|s| *s == settings.style)
                .unwrap_or(0) as u32,
        )
        .build();
    style.connect_selected_notify({
        let ctx = ctx.clone();
        move |r| {
            let s = styles
                .get(r.selected() as usize)
                .copied()
                .unwrap_or(Style::System);
            ctx.update_settings(|st| st.style = s);
        }
    });
    let appearance = adw::PreferencesGroup::builder().title("Appearance").build();
    appearance.add(&style);

    let page = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("preferences-system-symbolic")
        .build();
    page.add(&updates);
    page.add(&scanning);
    page.add(&appearance);
    let dialog = adw::PreferencesDialog::new();
    dialog.add(&page);
    dialog.present(Some(&ctx.window));
}
