// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Wire formats for the printer and scanner protocols used by spool.
//!
//! This crate performs no IO. Every parser here is tested against fixtures
//! recorded from real devices (see `tests/fixtures/README.md`).

pub mod cups;
pub mod dnssd;
pub mod escl;
pub mod ipp;
pub mod jobs;
pub mod printer;
pub mod uri;
