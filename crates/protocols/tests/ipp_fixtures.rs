// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Decoding of IPP responses recorded from real devices (see fixtures/README.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use spool_protocols::cups::Queue;
use spool_protocols::ipp::{GroupTag, Message, Response, StatusCode};
use spool_protocols::jobs::Job;
use spool_protocols::printer::{
    Level, MarkerKind, PrinterState, PrinterStatus, Severity, SupplyWarning,
};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn printer_attributes_from_hp_envy_5530() {
    let resp = Response::decode(&fixture("printer-get-printer-attributes.ipp")).unwrap();
    assert_eq!(resp.status, StatusCode::OK);
    let printer = resp.first_group(GroupTag::Printer).unwrap();
    let st = PrinterStatus::from_attributes(printer);
    assert_eq!(st.state, PrinterState::Idle);
    assert!(st.reasons.is_empty());
    assert_eq!(
        st.info.make_and_model.as_deref(),
        Some("HP ENVY 5530 series")
    );
    assert_eq!(
        st.info.more_info.as_deref(),
        Some("http://192.168.1.147/#hId-pgAirPrint")
    );
    assert_eq!(st.markers.len(), 2);
    let tri = &st.markers[0];
    assert_eq!(tri.name, "tri-color ink");
    assert_eq!(tri.colors.len(), 3);
    assert_eq!(tri.kind, MarkerKind::Ink);
    assert_eq!(tri.level, Level::Percent(30));
    assert_eq!(tri.low_level, Some(1));
    assert_eq!(tri.warning, None);
    let black = &st.markers[1];
    assert_eq!(black.label(), "black");
    assert_eq!(black.level, Level::Percent(80));
    // This printer exposes no trays and no media-col-ready.
    assert!(st.trays.is_empty());
    assert!(st.notices().is_empty());
}

#[test]
fn cups_queue_attributes() {
    let resp = Response::decode(&fixture("cups-queue-get-printer-attributes.ipp")).unwrap();
    let st = PrinterStatus::from_attributes(resp.first_group(GroupTag::Printer).unwrap());
    assert_eq!(st.state, PrinterState::Idle);
    assert_eq!(st.markers.len(), 2);
    assert_eq!(st.accepting_jobs, Some(true));
    // CUPS reports PPD media as media-ready keywords.
    assert!(st.media_ready.iter().any(|m| m.size == "A4"));
}

#[test]
fn cups_get_printers() {
    let resp = Response::decode(&fixture("cups-get-printers.ipp")).unwrap();
    let queues = Queue::list_from(&resp);
    // The recording machine has a manual queue and one created by cups-browsed.
    assert_eq!(queues.len(), 2);
    let q = &queues[0];
    assert_eq!(q.name, "HP_ENVY_5530");
    assert_eq!(q.device_host().as_deref(), Some("192.168.1.147"));
    assert!(!q.is_hplip());
    assert!(!q.is_class);
    let browsed = &queues[1];
    assert_eq!(browsed.name, "Vivek_Home_Printer");
    assert_eq!(
        browsed.device_uri.as_deref(),
        Some("implicitclass://Vivek_Home_Printer/")
    );
    assert_eq!(browsed.device_host(), None);
}

#[test]
fn cups_jobs() {
    let resp = Response::decode(&fixture("cups-get-jobs-completed.ipp")).unwrap();
    let jobs = Job::list_from(&resp);
    assert!(!jobs.is_empty());
    for j in &jobs {
        assert!(j.state.is_finished(), "{j:?}");
        assert!(j.actions().retry);
        assert!(j.created.is_some());
    }
    let resp = Response::decode(&fixture("cups-get-jobs-not-completed.ipp")).unwrap();
    assert!(Job::list_from(&resp).is_empty());
}

#[test]
fn cups_errors() {
    let resp = Response::decode(&fixture("cups-not-found.ipp")).unwrap();
    assert_eq!(resp.status, StatusCode::NOT_FOUND);
    assert!(resp.status_message().is_some());
    let resp = Response::decode(&fixture("cups-get-default.ipp")).unwrap();
    assert_eq!(resp.status, StatusCode::NOT_FOUND);
}

#[test]
fn fixtures_re_encode_identically() {
    for f in [
        "printer-get-printer-attributes.ipp",
        "cups-queue-get-printer-attributes.ipp",
        "cups-get-printers.ipp",
        "cups-get-jobs-completed.ipp",
    ] {
        let bytes = fixture(f);
        let m = Message::decode(&bytes).unwrap();
        assert_eq!(m.encode(), bytes, "{f}");
    }
}

#[test]
fn low_supply_reason_is_replaced_by_marker_sentence() {
    use spool_protocols::ipp::{Attribute, Value};
    let attrs = vec![
        Attribute::new("printer-state", Value::Enum(3)),
        Attribute::with_values(
            "printer-state-reasons",
            vec![Value::keyword("marker-supply-low-warning")],
        ),
        Attribute::new("marker-names", Value::name("Cyan Cartridge")),
        Attribute::new("marker-colors", Value::name("#00FFFF")),
        Attribute::new("marker-levels", Value::Integer(12)),
        Attribute::new("marker-low-levels", Value::Integer(15)),
        Attribute::new("marker-types", Value::keyword("ink-cartridge")),
    ];
    let st = PrinterStatus::from_attributes(&attrs);
    assert_eq!(st.markers[0].warning, Some(SupplyWarning::Low));
    let n = st.notices();
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].text, "Ink is low (cyan 12%)");
    assert_eq!(n[0].severity, Severity::Warning);
}
