// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! DNS-SD service records to printer and scanner endpoints.
//!
//! Printers: `_ipp._tcp` and `_ipps._tcp` with TXT keys from PWG 5100.14 section
//! 4.2.4, Table 3 (rp, UUID, ty, adminurl, note, pdl, Color). Scanners:
//! `_uscan._tcp` and `_uscans._tcp` with eSCL TXT keys (rs, UUID, ty, adminurl,
//! note, is, cs, duplex; as decoded by OpenPrinting go-mfp discovery/dnssd/txt.go).
//! TXT keys are case-insensitive (RFC 6763 section 6.4).

use crate::uri::host_for_uri;
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceKind {
    Ipp,
    Ipps,
    Uscan,
    Uscans,
}

impl ServiceKind {
    pub const ALL: [ServiceKind; 4] = [Self::Ipp, Self::Ipps, Self::Uscan, Self::Uscans];

    /// Fully qualified service type for browsing.
    pub fn service_type(self) -> &'static str {
        match self {
            Self::Ipp => "_ipp._tcp.local.",
            Self::Ipps => "_ipps._tcp.local.",
            Self::Uscan => "_uscan._tcp.local.",
            Self::Uscans => "_uscans._tcp.local.",
        }
    }

    pub fn from_service_type(t: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.service_type() == t)
    }

    pub fn is_tls(self) -> bool {
        matches!(self, Self::Ipps | Self::Uscans)
    }

    pub fn is_printer(self) -> bool {
        matches!(self, Self::Ipp | Self::Ipps)
    }
}

/// A resolved DNS-SD service, as handed over by the mDNS layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRecord {
    pub kind: ServiceKind,
    /// Instance name, for example "HP ENVY 5530 series [6A3607]".
    pub instance: String,
    /// Target host name, for example "HP6451066A3607.local.".
    pub hostname: String,
    pub port: u16,
    pub addresses: Vec<IpAddr>,
    pub txt: Vec<(String, String)>,
}

impl ServiceRecord {
    pub fn txt(&self, key: &str) -> Option<&str> {
        self.txt
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    }

    /// Host to put in URIs. An IPv4 literal is preferred: it works without
    /// nss-mdns, which the Flatpak sandbox and many systems lack. Link-local IPv6
    /// needs a zone, so it is the last choice before the host name.
    pub fn best_host(&self) -> String {
        if let Some(v4) = self.addresses.iter().find(|a| a.is_ipv4()) {
            return v4.to_string();
        }
        if let Some(v6) = self.addresses.iter().find(|a| match a {
            IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80,
            IpAddr::V4(_) => false,
        }) {
            return v6.to_string();
        }
        self.hostname.trim_end_matches('.').to_string()
    }

    /// Device UUID without "urn:uuid:", lowercase, used to merge the printer and
    /// scanner halves of one device.
    pub fn uuid(&self) -> Option<String> {
        self.txt("UUID").map(normalize_uuid)
    }
}

pub fn normalize_uuid(s: &str) -> String {
    s.trim()
        .trim_start_matches("urn:uuid:")
        .to_ascii_lowercase()
}

fn yes_no(v: Option<&str>) -> Option<bool> {
    match v? {
        "T" | "t" => Some(true),
        "F" | "f" => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterService {
    pub instance: String,
    pub uri: String,
    pub host: String,
    pub uuid: Option<String>,
    pub make_and_model: Option<String>,
    pub admin_url: Option<String>,
    pub location: Option<String>,
    pub tls: bool,
    pub color: Option<bool>,
}

impl PrinterService {
    pub fn from_record(r: &ServiceRecord) -> Option<Self> {
        if !r.kind.is_printer() {
            return None;
        }
        let scheme = if r.kind.is_tls() { "ipps" } else { "ipp" };
        // rp default is '' (PWG 5100.14 Table 3), meaning the root resource.
        let rp = r.txt("rp").unwrap_or("").trim_matches('/');
        let host = r.best_host();
        Some(Self {
            instance: r.instance.clone(),
            uri: format!("{scheme}://{}:{}/{rp}", host_for_uri(&host), r.port),
            host,
            uuid: r.uuid(),
            make_and_model: r.txt("ty").map(str::to_string),
            admin_url: r.txt("adminurl").map(str::to_string),
            location: r.txt("note").map(str::to_string),
            tls: r.kind.is_tls(),
            color: yes_no(r.txt("Color")),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannerService {
    pub instance: String,
    /// Base URL ending in "/", for example "http://192.168.1.147:80/eSCL/".
    pub base_url: String,
    pub host: String,
    pub uuid: Option<String>,
    pub make_and_model: Option<String>,
    pub admin_url: Option<String>,
    pub location: Option<String>,
    pub tls: bool,
    /// "is" key: input sources such as "platen,adf".
    pub inputs: Vec<String>,
    pub duplex: Option<bool>,
}

impl ScannerService {
    pub fn from_record(r: &ServiceRecord) -> Option<Self> {
        if r.kind.is_printer() {
            return None;
        }
        let scheme = if r.kind.is_tls() { "https" } else { "http" };
        let rs = r.txt("rs").unwrap_or("eSCL").trim_matches('/');
        let host = r.best_host();
        let path = if rs.is_empty() {
            "/".to_string()
        } else {
            format!("/{rs}/")
        };
        Some(Self {
            instance: r.instance.clone(),
            base_url: format!("{scheme}://{}:{}{path}", host_for_uri(&host), r.port),
            host,
            uuid: r.uuid(),
            make_and_model: r.txt("ty").map(str::to_string),
            admin_url: r.txt("adminurl").map(str::to_string),
            location: r.txt("note").map(str::to_string),
            tls: r.kind.is_tls(),
            inputs: r
                .txt("is")
                .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default(),
            duplex: yes_no(r.txt("duplex")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(kind: ServiceKind, txt: &[(&str, &str)], addrs: &[&str]) -> ServiceRecord {
        ServiceRecord {
            kind,
            instance: "Office".into(),
            hostname: "office.local.".into(),
            port: if kind.is_printer() { 631 } else { 80 },
            addresses: addrs.iter().map(|a| a.parse().unwrap()).collect(),
            txt: txt
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn printer_uri_from_rp() {
        let r = rec(
            ServiceKind::Ipp,
            &[
                ("rp", "ipp/print"),
                ("UUID", "1C852A4D-B800"),
                ("ty", "HP ENVY"),
            ],
            &["fe80::1", "192.168.1.147"],
        );
        let p = PrinterService::from_record(&r).unwrap();
        assert_eq!(p.uri, "ipp://192.168.1.147:631/ipp/print");
        assert_eq!(p.uuid.as_deref(), Some("1c852a4d-b800"));
        assert_eq!(p.make_and_model.as_deref(), Some("HP ENVY"));
    }

    #[test]
    fn missing_rp_means_root() {
        let r = rec(ServiceKind::Ipps, &[], &["10.0.0.2"]);
        assert_eq!(
            PrinterService::from_record(&r).unwrap().uri,
            "ipps://10.0.0.2:631/"
        );
    }

    #[test]
    fn scanner_base_url_defaults_to_escl() {
        let r = rec(
            ServiceKind::Uscan,
            &[("is", "platen,adf"), ("duplex", "T")],
            &["10.0.0.3"],
        );
        let s = ScannerService::from_record(&r).unwrap();
        assert_eq!(s.base_url, "http://10.0.0.3:80/eSCL/");
        assert_eq!(s.inputs, vec!["platen", "adf"]);
        assert_eq!(s.duplex, Some(true));
        let r = rec(ServiceKind::Uscans, &[("RS", "/scan/")], &["2001:db8::5"]);
        let s = ScannerService::from_record(&r).unwrap();
        assert_eq!(s.base_url, "https://[2001:db8::5]:80/scan/");
    }

    #[test]
    fn host_name_when_only_link_local() {
        let r = rec(ServiceKind::Ipp, &[], &["fe80::1"]);
        assert_eq!(r.best_host(), "office.local");
    }

    #[test]
    fn kinds_do_not_cross() {
        let r = rec(ServiceKind::Uscan, &[], &["10.0.0.3"]);
        assert!(PrinterService::from_record(&r).is_none());
    }
}
