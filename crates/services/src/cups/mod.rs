// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! CUPS queues, jobs and administration over IPP.

pub mod client;
pub mod testpage;

#[cfg(test)]
mod live_tests;

pub use client::{CupsClient, CupsError, JobOp, NewQueue, WhichJobs};
