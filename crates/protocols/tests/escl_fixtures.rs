// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! eSCL documents recorded from real scanners (see fixtures/README.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use spool_protocols::escl::{
    ColorMode, InputSource, PaperSize, ScanRequest, ScanSettings, ScannerCapabilities,
    ScannerState, ScannerStatus, SettingsError,
};

fn request(source: InputSource, resolution: u32, format: &str) -> ScanRequest {
    ScanRequest {
        source,
        duplex: false,
        color_mode: ColorMode::Rgb24,
        resolution,
        paper: PaperSize::A4,
        document_format: format.into(),
        intent: None,
    }
}

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn envy_5530_capabilities() {
    let c = ScannerCapabilities::parse(&fixture("escl-ScannerCapabilities.xml")).unwrap();
    assert_eq!(c.version, "2.1");
    assert_eq!(c.version_tuple(), (2, 1));
    assert_eq!(c.sources(), vec![InputSource::Platen]);
    let p = c.platen.as_ref().unwrap();
    assert_eq!((p.max_width, p.max_height), (2550, 3508));
    assert_eq!(p.color_modes, vec![ColorMode::Rgb24, ColorMode::Grayscale8]);
    assert_eq!(p.resolutions, vec![75, 100, 200, 300, 600, 1200]);
    assert!(p.supports_format("image/jpeg"));
    assert!(p.supports_format("application/pdf"));
    assert!(PaperSize::A4.fits(p));
    assert!(PaperSize::Letter.fits(p));
    assert!(!PaperSize::Legal.fits(p));
}

#[test]
fn envy_5530_status() {
    let s = ScannerStatus::parse(&fixture("escl-ScannerStatus.xml")).unwrap();
    assert_eq!(s.state, ScannerState::Idle);
    assert!(s.adf_state.is_none());
}

#[test]
fn m426fdn_adf_capabilities_and_status() {
    let c = ScannerCapabilities::parse(&fixture(
        "escl-thirdparty-hp-m426fdn-ScannerCapabilities.xml",
    ))
    .unwrap();
    assert_eq!(c.version_tuple(), (2, 5));
    assert_eq!(c.sources(), vec![InputSource::Platen, InputSource::Feeder]);
    assert_eq!(c.feeder_capacity, Some(50));
    let duplex = c.input(InputSource::Feeder, true).unwrap();
    assert_eq!(duplex.resolutions, vec![75, 200, 300]);
    assert_eq!(duplex.max_height, 4500);
    assert!(PaperSize::Legal.fits(duplex));
    let s = ScannerStatus::parse(&fixture("escl-thirdparty-hp-m426fdn-ScannerStatus.xml")).unwrap();
    assert_eq!(s.adf_has_paper(), Some(false));
    assert_eq!(s.adf_sentence(), Some("The document feeder is empty."));
    assert_eq!(s.jobs.len(), 2);
    assert_eq!(s.jobs[0].uri, "/eSCL/ScanJobs/1005");
}

#[test]
fn scan_settings_xml_follows_schema_order() {
    let c = ScannerCapabilities::parse(&fixture("escl-ScannerCapabilities.xml")).unwrap();
    let s =
        ScanSettings::for_request(&c, &request(InputSource::Platen, 300, "image/jpeg")).unwrap();
    let xml = s.to_xml();
    let order = [
        "<pwg:Version>2.1</pwg:Version>",
        "<pwg:XOffset>0</pwg:XOffset>",
        "<pwg:Width>2480</pwg:Width>",
        "<pwg:Height>3508</pwg:Height>",
        "escl:ThreeHundredthsOfInches",
        "<pwg:DocumentFormat>image/jpeg</pwg:DocumentFormat>",
        "<scan:DocumentFormatExt>image/jpeg</scan:DocumentFormatExt>",
        "<pwg:InputSource>Platen</pwg:InputSource>",
        "<scan:XResolution>300</scan:XResolution>",
        "<scan:YResolution>300</scan:YResolution>",
        "<scan:ColorMode>RGB24</scan:ColorMode>",
    ];
    let mut at = 0;
    for o in order {
        let i = xml[at..]
            .find(o)
            .unwrap_or_else(|| panic!("{o} missing after {at}:\n{xml}"));
        at += i + o.len();
    }
    assert!(!xml.contains("Duplex"));
    // The document parses as ScanSettings with our own reader.
    let e = spool_protocols::escl::xml::Element::parse(&xml).unwrap();
    assert_eq!(e.name, "ScanSettings");
}

#[test]
fn full_area_uses_input_maximum() {
    let c = ScannerCapabilities::parse(&fixture("escl-ScannerCapabilities.xml")).unwrap();
    let r = PaperSize::Full.region(c.platen.as_ref().unwrap());
    assert_eq!((r.width, r.height), (2550, 3508));
}

#[test]
fn wrong_document_is_rejected() {
    assert!(ScannerCapabilities::parse(&fixture("escl-ScannerStatus.xml")).is_err());
    assert!(ScannerStatus::parse("<x/>").is_err());
}

#[test]
fn requests_are_validated_against_capabilities() {
    let c = ScannerCapabilities::parse(&fixture("escl-ScannerCapabilities.xml")).unwrap();
    assert_eq!(
        ScanSettings::for_request(&c, &request(InputSource::Feeder, 300, "image/jpeg")),
        Err(SettingsError::SourceNotSupported)
    );
    assert_eq!(
        ScanSettings::for_request(&c, &request(InputSource::Platen, 150, "image/jpeg")),
        Err(SettingsError::ResolutionNotSupported(150))
    );
    assert_eq!(
        ScanSettings::for_request(&c, &request(InputSource::Platen, 300, "image/png")),
        Err(SettingsError::FormatNotSupported("image/png".into()))
    );
    let mut legal = request(InputSource::Platen, 300, "image/jpeg");
    legal.paper = PaperSize::Legal;
    assert_eq!(
        ScanSettings::for_request(&c, &legal),
        Err(SettingsError::PaperTooLarge)
    );
}

#[test]
fn feeder_duplex_is_sent_for_adf() {
    let c = ScannerCapabilities::parse(&fixture(
        "escl-thirdparty-hp-m426fdn-ScannerCapabilities.xml",
    ))
    .unwrap();
    let mut r = request(InputSource::Feeder, 300, "image/jpeg");
    r.duplex = true;
    let xml = ScanSettings::for_request(&c, &r).unwrap().to_xml();
    assert!(xml.contains("<pwg:InputSource>Feeder</pwg:InputSource>"));
    assert!(xml.contains("<scan:Duplex>true</scan:Duplex>"));
    assert!(xml.contains("<pwg:Version>2.5</pwg:Version>"));
}
