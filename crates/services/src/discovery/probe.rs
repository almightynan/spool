// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Adding a device by IP address, host name or ipp:// URI.
//!
//! IPP: Get-Printer-Attributes on the given ipp(s) URI, or on
//! ipp://host:631/ipp/print (PWG 5100.14 section 5.4.3 recommends /ipp/print).
//! eSCL: GET http://host/eSCL/ScannerCapabilities ("eSCL" is the default "rs").
//! Both run at the same time; the device exists if either answers.

use crate::ipp_client::{self, IppTarget};
use crate::model::{NetDevice, Origin};
use crate::net::{self, CancelToken, HttpRequest, Limits, NetError};
use hyper::Method;
use spool_protocols::dnssd::{normalize_uuid, PrinterService, ScannerService};
use spool_protocols::escl::ScannerCapabilities;
use spool_protocols::ipp::{AttributeSet, GroupTag, Operation, Request};
use spool_protocols::uri::{host_for_uri, Uri};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    InvalidAddress,
    /// Nothing answered at all.
    Unreachable(NetError),
    /// The host answered, but neither IPP nor eSCL.
    NothingFound,
    Cancelled,
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddress => write!(f, "enter an IP address, host name or ipp:// address"),
            Self::Unreachable(e) => e.fmt(f),
            Self::NothingFound => write!(f, "no IPP printer or eSCL scanner answered there"),
            Self::Cancelled => write!(f, "canceled"),
        }
    }
}

/// Splits user input into a host and an optional explicit printer URI.
pub fn parse_input(input: &str) -> Option<(String, Option<String>)> {
    let s = input.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    if s.contains("://") {
        let u = Uri::parse(s)?;
        if !matches!(u.scheme.as_str(), "ipp" | "ipps") || u.host.is_empty() {
            return None;
        }
        return Some((u.host.clone(), Some(s.to_string())));
    }
    let host = s.trim_start_matches('[').trim_end_matches(']');
    let ok = host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '%' | '_'));
    ok.then(|| (host.to_string(), None))
}

pub async fn probe(input: &str, user: &str, cancel: &CancelToken) -> Result<NetDevice, ProbeError> {
    let (host, explicit) = parse_input(input).ok_or(ProbeError::InvalidAddress)?;
    let printer_uri =
        explicit.unwrap_or_else(|| format!("ipp://{}:631/ipp/print", host_for_uri(&host)));
    let scanner_url = format!("http://{}:80/eSCL/", host_for_uri(&host));
    let (ipp, escl) = tokio::join!(
        probe_ipp(&printer_uri, user, cancel),
        probe_escl(&scanner_url, cancel)
    );
    if cancel.is_cancelled() {
        return Err(ProbeError::Cancelled);
    }
    let ipp_err = ipp.as_ref().err().cloned();
    let (printer, info) = match ipp {
        Ok((p, i)) => (Some(p), i),
        Err(_) => (None, PrinterInfo::default()),
    };
    let scanner = match escl {
        Ok(s) => Some(s),
        Err(escl_err) => {
            if printer.is_none() {
                // Unreachable only when neither service got an answer.
                return Err(match ipp_err {
                    Some(e) if e.is_unreachable() && escl_err.is_unreachable() => {
                        ProbeError::Unreachable(escl_err)
                    }
                    _ => ProbeError::NothingFound,
                });
            }
            None
        }
    };
    let make = info
        .make_and_model
        .clone()
        .or_else(|| scanner.as_ref().and_then(|s| s.make_and_model.clone()));
    Ok(NetDevice {
        key: info
            .uuid
            .clone()
            .or_else(|| scanner.as_ref().and_then(|s| s.uuid.clone()))
            .unwrap_or_else(|| host.clone()),
        name: info
            .name
            .clone()
            .or_else(|| make.clone())
            .unwrap_or_else(|| host.clone()),
        host,
        make_and_model: make,
        location: info.location.clone(),
        admin_url: info.more_info.clone(),
        printer,
        scanner,
        origin: Origin::Manual,
        unreachable: false,
        saved_address: Some(input.trim().to_string()),
    })
}

#[derive(Default)]
struct PrinterInfo {
    name: Option<String>,
    make_and_model: Option<String>,
    uuid: Option<String>,
    location: Option<String>,
    more_info: Option<String>,
}

async fn probe_ipp(
    uri: &str,
    user: &str,
    cancel: &CancelToken,
) -> Result<(PrinterService, PrinterInfo), ipp_client::IppError> {
    let target = IppTarget::for_uri(uri)?;
    let req = Request::new(Operation::GET_PRINTER_ATTRIBUTES)
        .printer_uri(uri)
        .user(user)
        .requested_attributes(&[
            "printer-name",
            "printer-info",
            "printer-make-and-model",
            "printer-uuid",
            "printer-location",
            "printer-more-info",
            "printer-dns-sd-name",
        ]);
    let resp = ipp_client::send(&target, req, None, Limits::QUERY, cancel).await?;
    let g = resp.first_group(GroupTag::Printer);
    let s = |n: &str| {
        g.and_then(|g| g.str_of(n))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let info = PrinterInfo {
        name: s("printer-dns-sd-name").or_else(|| s("printer-info")),
        make_and_model: s("printer-make-and-model"),
        uuid: s("printer-uuid").map(|u| normalize_uuid(&u)),
        location: s("printer-location"),
        more_info: s("printer-more-info"),
    };
    let u = Uri::parse(uri);
    let service = PrinterService {
        instance: info.name.clone().unwrap_or_default(),
        uri: uri.to_string(),
        host: u.map(|u| u.host).unwrap_or_default(),
        uuid: info.uuid.clone(),
        make_and_model: info.make_and_model.clone(),
        admin_url: info.more_info.clone(),
        location: info.location.clone(),
        tls: uri.starts_with("ipps:"),
        color: None,
    };
    Ok((service, info))
}

async fn probe_escl(base: &str, cancel: &CancelToken) -> Result<ScannerService, NetError> {
    let req = HttpRequest::to_url(Method::GET, &format!("{base}ScannerCapabilities"))?;
    let r = net::send(req, Limits::QUERY, cancel).await?;
    if r.status != 200 {
        return Err(NetError::Protocol(format!("HTTP {}", r.status)));
    }
    let text = String::from_utf8_lossy(&r.body);
    let caps = ScannerCapabilities::parse(&text).map_err(|e| NetError::Protocol(e.to_string()))?;
    let host = Uri::parse(base).map(|u| u.host).unwrap_or_default();
    Ok(ScannerService {
        instance: caps.make_and_model.clone().unwrap_or_else(|| host.clone()),
        base_url: base.to_string(),
        host,
        uuid: caps.uuid.as_deref().map(normalize_uuid),
        make_and_model: caps.make_and_model.clone(),
        admin_url: caps.admin_uri.clone(),
        location: None,
        tls: false,
        inputs: Vec::new(),
        duplex: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_parsing() {
        assert_eq!(
            parse_input(" 192.168.1.147 "),
            Some(("192.168.1.147".into(), None))
        );
        assert_eq!(
            parse_input("printer.local"),
            Some(("printer.local".into(), None))
        );
        assert_eq!(
            parse_input("[fe80::1%wlan0]"),
            Some(("fe80::1%wlan0".into(), None))
        );
        assert_eq!(
            parse_input("ipp://10.0.0.5:8631/ipp/print"),
            Some((
                "10.0.0.5".into(),
                Some("ipp://10.0.0.5:8631/ipp/print".into())
            ))
        );
        assert_eq!(parse_input("http://10.0.0.5/"), None);
        assert_eq!(parse_input("two words"), None);
        assert_eq!(parse_input(""), None);
        assert_eq!(parse_input("a/b"), None);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// `SPOOL_TEST_HOST=192.168.1.147 cargo test -p spool-services probe_real -- --ignored`
    #[tokio::test]
    #[ignore = "needs a printer on the network"]
    async fn probe_real_device() {
        let Ok(host) = std::env::var("SPOOL_TEST_HOST") else {
            return;
        };
        let d = probe(&host, "tester", &CancelToken::new()).await.unwrap();
        eprintln!("{d:#?}");
        assert!(d.printer.is_some() || d.scanner.is_some());
    }

    #[tokio::test]
    async fn unreachable_host() {
        // TEST-NET-1 (RFC 5737) is never routed.
        let e = probe("192.0.2.1", "tester", &CancelToken::new())
            .await
            .unwrap_err();
        assert!(matches!(e, ProbeError::Unreachable(_)), "{e:?}");
    }
}
