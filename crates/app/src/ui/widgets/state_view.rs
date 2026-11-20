// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! A stack that shows one of: loading, empty, error, offline, or the content.
//! All states share one allocation, so switching never moves the layout.

use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

type Callback = Rc<RefCell<Option<Box<dyn Fn()>>>>;

#[derive(Clone)]
pub struct StateView {
    stack: gtk::Stack,
    loading: adw::StatusPage,
    message: adw::StatusPage,
    retry: gtk::Button,
    on_retry: Callback,
    action_button: gtk::Button,
    on_action: Callback,
}

impl StateView {
    pub fn new(content: &impl IsA<gtk::Widget>) -> Self {
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .vexpand(true)
            .hexpand(true)
            .build();
        let loading = adw::StatusPage::new();
        // The paintable animates on the frame clock of the page that shows it.
        loading.set_paintable(Some(&adw::SpinnerPaintable::new(Some(&loading))));

        let retry = gtk::Button::builder()
            .label("Try Again")
            .halign(gtk::Align::Center)
            .css_classes(["pill"])
            .build();
        let action_button = gtk::Button::builder()
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .visible(false)
            .build();
        let buttons = gtk::Box::new(gtk::Orientation::Vertical, 12);
        buttons.append(&action_button);
        buttons.append(&retry);
        let message = adw::StatusPage::builder().child(&buttons).build();

        stack.add_named(&loading, Some("loading"));
        stack.add_named(&message, Some("message"));
        stack.add_named(content, Some("content"));

        let on_retry: Callback = Rc::new(RefCell::new(None));
        let on_action: Callback = Rc::new(RefCell::new(None));
        retry.connect_clicked({
            let cb = on_retry.clone();
            move |_| {
                if let Some(f) = cb.borrow().as_ref() {
                    f();
                }
            }
        });
        action_button.connect_clicked({
            let cb = on_action.clone();
            move |_| {
                if let Some(f) = cb.borrow().as_ref() {
                    f();
                }
            }
        });
        Self {
            stack,
            loading,
            message,
            retry,
            on_retry,
            action_button,
            on_action,
        }
    }

    pub fn widget(&self) -> &gtk::Stack {
        &self.stack
    }

    pub fn set_retry(&self, f: impl Fn() + 'static) {
        *self.on_retry.borrow_mut() = Some(Box::new(f));
    }

    pub fn loading(&self, title: &str) {
        self.loading.set_title(title);
        self.stack.set_visible_child_name("loading");
    }

    pub fn content(&self) {
        self.stack.set_visible_child_name("content");
    }

    pub fn is_content(&self) -> bool {
        self.stack.visible_child_name().as_deref() == Some("content")
    }

    fn show_message(&self, icon: &str, title: &str, description: &str, retry: bool) {
        self.message.set_icon_name(Some(icon));
        self.message.set_title(title);
        self.message.set_description(Some(description));
        self.retry.set_visible(retry);
        self.action_button.set_visible(false);
        self.stack.set_visible_child_name("message");
    }

    /// Empty state; no retry button.
    pub fn empty(&self, icon: &str, title: &str, description: &str) {
        self.show_message(icon, title, description, false);
    }

    pub fn error(&self, title: &str, description: &str) {
        self.show_message("dialog-error-symbolic", title, description, true);
    }

    pub fn offline(&self, title: &str, description: &str) {
        self.show_message("network-offline-symbolic", title, description, true);
    }

    /// Message with one main action instead of a retry button.
    pub fn with_action(
        &self,
        icon: &str,
        title: &str,
        description: &str,
        label: &str,
        f: impl Fn() + 'static,
    ) {
        self.show_message(icon, title, description, false);
        self.action_button.set_label(label);
        self.action_button.set_visible(true);
        *self.on_action.borrow_mut() = Some(Box::new(f));
    }
}
