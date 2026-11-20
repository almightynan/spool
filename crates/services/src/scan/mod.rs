// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Scanning over eSCL and saving the result.

pub mod jpeg;
pub mod pdf;
pub mod save;
pub mod session;

pub use save::{save_scan, SaveError, SaveFormat};
pub use session::{ScanClient, ScanError, ScanProgress, ScannedPage};
