// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! A button that shows a spinner and is insensitive while its work runs.
//! Label and spinner live in one homogeneous stack, so the width stays put.

use adw::prelude::*;

#[derive(Clone)]
pub struct BusyButton {
    button: gtk::Button,
    stack: gtk::Stack,
}

impl BusyButton {
    pub fn new(label: &str) -> Self {
        let stack = gtk::Stack::builder()
            .hhomogeneous(true)
            .vhomogeneous(true)
            .build();
        let text = gtk::Label::with_mnemonic(label);
        stack.add_named(&text, Some("label"));
        stack.add_named(&adw::Spinner::new(), Some("busy"));
        let button = gtk::Button::builder().child(&stack).build();
        // Mnemonic labels inside a custom child need the button as target.
        text.set_mnemonic_widget(Some(&button));
        button.update_property(&[gtk::accessible::Property::Label(&label.replace('_', ""))]);
        Self { button, stack }
    }

    /// Icon-only button with a tooltip that doubles as the accessible label.
    pub fn icon(icon: &str, tooltip: &str) -> Self {
        let stack = gtk::Stack::builder()
            .hhomogeneous(true)
            .vhomogeneous(true)
            .build();
        stack.add_named(&gtk::Image::from_icon_name(icon), Some("label"));
        stack.add_named(&adw::Spinner::new(), Some("busy"));
        let button = gtk::Button::builder()
            .child(&stack)
            .tooltip_text(tooltip)
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
        Self { button, stack }
    }

    pub fn widget(&self) -> &gtk::Button {
        &self.button
    }

    pub fn set_busy(&self, busy: bool) {
        self.stack
            .set_visible_child_name(if busy { "busy" } else { "label" });
        self.button.set_sensitive(!busy);
        self.button
            .update_state(&[gtk::accessible::State::Busy(busy)]);
    }

    pub fn is_busy(&self) -> bool {
        self.stack.visible_child_name().as_deref() == Some("busy")
    }

    pub fn connect_clicked(&self, f: impl Fn() + 'static) {
        self.button.connect_clicked(move |_| f());
    }
}
