// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Finding printers and scanners: DNS-SD, manual addresses, and the registry
//! that merges them with CUPS queues.

pub mod mdns;
pub mod probe;
pub mod registry;

pub use probe::ProbeError;
pub use registry::{spawn_registry, Registry, RegistryError};
