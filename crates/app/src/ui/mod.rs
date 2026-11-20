// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! GTK widgets. Nothing here performs IO; all work goes through
//! `runtime::Worker`.

pub mod actions;
pub mod add_device_dialog;
pub mod add_queue_dialog;
pub mod dashboard;
pub mod device_page;
pub mod jobs_page;
pub mod maintenance_page;
pub mod preferences;
pub mod scan_page;
pub mod sidebar;
pub mod text;
pub mod widgets;
pub mod window;
