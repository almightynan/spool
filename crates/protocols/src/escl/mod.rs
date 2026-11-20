// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! eSCL (AirScan) documents: capabilities, status and scan settings.
//!
//! Wire format: Mopria eSCL Technical Specification (sections 7 to 9), as cited
//! by OpenPrinting go-mfp proto/escl. Endpoints, relative to the "rs" resource
//! path from DNS-SD (default "eSCL"):
//! - GET  ScannerCapabilities, GET ScannerStatus
//! - POST ScanJobs (ScanSettings) answers 201 with Location of the job
//! - GET  {job}/NextDocument answers 200 per page, 404 when no pages remain
//! - DELETE {job} cancels
//! - 503 means the scanner is busy, 409 that the settings conflict with its state

pub mod caps;
pub mod settings;
pub mod status;
pub mod xml;

pub use caps::{ColorMode, InputCaps, InputSource, ScannerCapabilities};
pub use settings::{PaperSize, Region, ScanRequest, ScanSettings, SettingsError};
pub use status::{JobInfo, ScannerState, ScannerStatus};
pub use xml::XmlError;
