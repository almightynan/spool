// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! IO services for spool. Everything here runs on the tokio worker runtime;
//! nothing here knows about GTK.

pub mod cups;
pub mod discovery;
pub mod hplip;
pub mod identity;
pub mod ipp_client;
pub mod model;
pub mod monitor;
pub mod net;
pub mod poll;
pub mod scan;
pub mod settings;

#[cfg(test)]
mod test_server;
