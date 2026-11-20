// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! CUPS queues from CUPS-Get-Printers (CUPS IPP spec, operation 0x4002).

use crate::ipp::{AttributeSet, Group, GroupTag, Response};
use crate::printer::{PrinterState, StateReason};
use crate::uri::Uri;

pub const QUEUE_ATTRIBUTES: &[&str] = &[
    "printer-name",
    "printer-uri-supported",
    "device-uri",
    "printer-info",
    "printer-location",
    "printer-make-and-model",
    "printer-state",
    "printer-state-reasons",
    "printer-is-accepting-jobs",
    "printer-type",
    "printer-uuid",
    "printer-more-info",
];

/// printer-type bit for classes (CUPS IPP spec, "printer-type" values).
const CUPS_PRINTER_CLASS: i32 = 0x0001;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    pub name: String,
    pub uri: String,
    pub device_uri: Option<String>,
    pub info: Option<String>,
    pub location: Option<String>,
    pub make_and_model: Option<String>,
    pub state: PrinterState,
    pub reasons: Vec<StateReason>,
    pub accepting_jobs: bool,
    pub is_class: bool,
}

impl Queue {
    pub fn from_group(g: &Group) -> Option<Self> {
        let name = g.str_of("printer-name")?.to_string();
        let uri = g
            .str_of("printer-uri-supported")
            .map(str::to_string)
            .unwrap_or_else(|| format!("ipp://localhost/printers/{name}"));
        let state = g
            .int_of("printer-state")
            .map(PrinterState::from_enum)
            .unwrap_or(PrinterState::Unknown(0));
        let stopped = state == PrinterState::Stopped;
        let s = |n: &str| g.str_of(n).filter(|v| !v.is_empty()).map(str::to_string);
        Some(Self {
            name,
            uri,
            device_uri: s("device-uri"),
            info: s("printer-info"),
            location: s("printer-location"),
            make_and_model: s("printer-make-and-model"),
            state,
            reasons: g
                .strs_of("printer-state-reasons")
                .into_iter()
                .filter_map(|r| StateReason::parse(r, stopped))
                .collect(),
            accepting_jobs: g.bool_of("printer-is-accepting-jobs").unwrap_or(true),
            is_class: g.int_of("printer-type").unwrap_or(0) & CUPS_PRINTER_CLASS != 0,
        })
    }

    pub fn list_from(resp: &Response) -> Vec<Self> {
        resp.groups_of(GroupTag::Printer)
            .filter_map(Self::from_group)
            .collect()
    }

    /// Display name: printer-info when set, else the queue name.
    pub fn display_name(&self) -> &str {
        self.info.as_deref().unwrap_or(&self.name)
    }

    /// Network host the queue prints to, for matching with discovered devices.
    pub fn device_host(&self) -> Option<String> {
        let u = Uri::parse(self.device_uri.as_deref()?)?;
        match u.scheme.as_str() {
            "ipp" | "ipps" | "http" | "https" | "socket" | "lpd" => Some(u.host),
            // HPLIP network URIs carry the address in the query: hp:/net/Model?ip=1.2.3.4
            "hp" | "hpfax" => u.query_param("ip").or_else(|| u.query_param("hostname")),
            _ => None,
        }
    }

    /// UUID from a dnssd:// device URI ("...?uuid=..."), for matching with mDNS.
    pub fn device_uuid(&self) -> Option<String> {
        let u = Uri::parse(self.device_uri.as_deref()?)?;
        if u.scheme == "dnssd" {
            u.query_param("uuid")
        } else {
            None
        }
    }

    /// True when the queue uses the HPLIP backend, which hp-clean and hp-align need.
    pub fn is_hplip(&self) -> bool {
        self.device_uri
            .as_deref()
            .is_some_and(|d| d.starts_with("hp:"))
    }
}

/// Name for a new queue made from a model name. Generated names use only ASCII
/// letters, digits, "-", "_" and ".", which always pass [`is_valid_queue_name`].
pub fn queue_name_from(model: &str) -> String {
    let mut out = String::new();
    for c in model.chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '-' | '.') {
            c
        } else {
            '_'
        };
        if c == '_' && out.ends_with('_') {
            continue;
        }
        out.push(c);
    }
    let trimmed: String = out.trim_matches('_').chars().take(127).collect();
    if trimmed.is_empty() {
        "Printer".to_string()
    } else {
        trimmed
    }
}

/// cupsd's rule (validate_name in cups 2.4 scheduler/ipp.c): no control
/// characters, space, DEL, "/" or "#", and shorter than 128 bytes.
pub fn is_valid_queue_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 128
        && !name
            .bytes()
            .any(|b| b <= b' ' || b == 127 || b == b'/' || b == b'#')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            queue_name_from("HP ENVY 5530 series"),
            "HP_ENVY_5530_series"
        );
        assert_eq!(queue_name_from("  #/ "), "Printer");
        assert!(is_valid_queue_name("HP_ENVY_5530"));
        assert!(!is_valid_queue_name("HP ENVY"));
        assert!(!is_valid_queue_name("a/b"));
        assert!(is_valid_queue_name("Büro?"));
    }
}
