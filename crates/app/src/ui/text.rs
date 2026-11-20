// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! User-facing wording. Plain, specific sentences; no em dashes.

use spool_protocols::printer::PrinterState;
use spool_services::cups::CupsError;
use spool_services::model::Device;
use spool_services::net::{ConnectFailure, NetError};
use std::fmt::Display;
use std::time::{SystemTime, UNIX_EPOCH};

/// "the printer is busy" -> "The printer is busy."
pub fn sentence(e: impl Display) -> String {
    let s = e.to_string();
    let mut c = s.chars();
    let mut out = match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => return String::new(),
    };
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

pub fn printer_state(s: PrinterState) -> &'static str {
    match s {
        PrinterState::Idle => "Ready",
        PrinterState::Processing => "Printing",
        PrinterState::Stopped => "Stopped",
        PrinterState::Unknown(_) => "State unknown",
    }
}

/// One-line summary for the sidebar row.
pub fn device_subtitle(d: &Device) -> String {
    if d.unreachable {
        return "Not responding".into();
    }
    if let Some(q) = &d.queue {
        if !q.accepting_jobs {
            return "Not accepting jobs".into();
        }
        return printer_state(q.state).into();
    }
    match (d.printer_uri.is_some(), d.scanner.is_some()) {
        (true, true) => "Printer and scanner, not added".into(),
        (true, false) => "Printer, not added".into(),
        (false, true) => "Scanner".into(),
        (false, false) => "No services found".into(),
    }
}

/// "192.168.1.147 did not answer within 10 seconds." for a device that did
/// not respond, naming the device once.
pub fn no_answer(who: &str, e: &NetError) -> String {
    match e {
        NetError::Timeout(d) => format!(
            "{who} did not answer within {} seconds.",
            d.as_secs().max(1)
        ),
        NetError::Connect {
            failure: ConnectFailure::Refused,
            ..
        } => format!("{who} refused the connection."),
        NetError::Connect {
            failure: ConnectFailure::UnknownHost,
            ..
        } => format!("{who} is not a known host name."),
        _ => format!("{who} cannot be reached."),
    }
}

/// Specific advice for CUPS errors.
pub fn cups_error(e: &CupsError) -> String {
    match e {
        CupsError::NotAllowed => {
            "Your account is not allowed to manage printers. Ask an administrator to add you to the lpadmin group.".into()
        }
        other => sentence(other),
    }
}

/// Local clock time "14:02" for UNIX seconds or a SystemTime.
pub fn clock(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    clock_unix(secs)
}

pub fn clock_unix(secs: i64) -> String {
    match gtk::glib::DateTime::from_unix_local(secs) {
        Ok(dt) => {
            let today = gtk::glib::DateTime::now_local()
                .map(|n| (n.year(), n.day_of_year()))
                .unwrap_or((0, 0));
            let fmt = if (dt.year(), dt.day_of_year()) == today {
                "%H:%M"
            } else {
                "%e %b, %H:%M"
            };
            dt.format(fmt)
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        }
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences() {
        assert_eq!(
            sentence("no response within 10 seconds"),
            "No response within 10 seconds."
        );
        assert_eq!(sentence("Done."), "Done.");
        assert_eq!(sentence(""), "");
    }
}
