// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! DNS-SD TXT records recorded from the HP ENVY 5530 (see fixtures/README.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use spool_protocols::dnssd::{PrinterService, ScannerService, ServiceKind, ServiceRecord};

/// Fixture format: "Kind<TAB>instance<TAB>host<TAB>port", then key=value lines.
fn record(name: &str) -> ServiceRecord {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().unwrap().split('\t').collect();
    let kind = match head[0] {
        "Ipp" => ServiceKind::Ipp,
        "Uscan" => ServiceKind::Uscan,
        k => panic!("{k}"),
    };
    ServiceRecord {
        kind,
        instance: head[1].into(),
        hostname: head[2].into(),
        port: head[3].parse().unwrap(),
        // Addresses are not part of TXT; these are the ones the device announced.
        addresses: vec![
            "fe80::6651:6ff:fe6a:3607".parse().unwrap(),
            "192.168.1.147".parse().unwrap(),
        ],
        txt: lines
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

#[test]
fn envy_ipp_record() {
    let r = record("dnssd-ipp-txt.txt");
    let p = PrinterService::from_record(&r).unwrap();
    assert_eq!(p.uri, "ipp://192.168.1.147:631/ipp/print");
    assert_eq!(
        p.uuid.as_deref(),
        Some("1c852a4d-b800-1f08-abcd-6451066a3607")
    );
    assert_eq!(p.make_and_model.as_deref(), Some("ENVY 5530 series"));
    assert_eq!(p.color, Some(true));
    assert_eq!(
        p.admin_url.as_deref(),
        Some("http://HP6451066A3607.local./#hId-pgAirPrint")
    );
    // Empty "note" means no location.
    assert_eq!(p.location, None);
}

#[test]
fn envy_uscan_record() {
    let r = record("dnssd-uscan-txt.txt");
    let s = ScannerService::from_record(&r).unwrap();
    assert_eq!(s.base_url, "http://192.168.1.147:8080/eSCL/");
    assert_eq!(s.inputs, vec!["platen"]);
    assert_eq!(s.duplex, Some(false));
    // Same UUID as the printer half: they are one device.
    assert_eq!(
        s.uuid,
        PrinterService::from_record(&record("dnssd-ipp-txt.txt"))
            .unwrap()
            .uuid
    );
}
