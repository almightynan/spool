// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! spool: manage network printers and scanners.

mod app;
mod runtime;
mod ui;

fn main() -> gtk::glib::ExitCode {
    app::application::run()
}
