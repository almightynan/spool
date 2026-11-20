// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Printer status model: state, state reasons, supplies, trays and ready media.

pub mod media;
pub mod model;
pub mod reasons;

pub use model::{
    Level, Marker, MarkerKind, MediaReady, Notice, PrinterInfo, PrinterState, PrinterStatus, Rgb,
    SubunitStatus, SupplyWarning, Tray, TrayLevel, STATUS_ATTRIBUTES,
};
pub use reasons::{Severity, StateReason};
