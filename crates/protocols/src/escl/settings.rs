// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! ScanSettings request body (Mopria eSCL Technical Specification section 7).
//! Element order follows the eSCL schema sequence, as implemented in OpenPrinting
//! go-mfp proto/escl/scansettings.go ToXML.

use super::caps::{ColorMode, InputCaps, InputSource, ScannerCapabilities};
use super::xml::escape;

pub const NS_SCAN: &str = "http://schemas.hp.com/imaging/escl/2011/05/03";
pub const NS_PWG: &str = "http://www.pwg.org/schemas/2010/12/sm";

/// Scan area in 1/300 inch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Paper sizes offered for scanning, with sizes in 1/300 inch (1 mm = 300/25.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperSize {
    /// The whole scan area the input reports.
    Full,
    A4,
    A5,
    Letter,
    Legal,
    Photo4x6,
}

impl PaperSize {
    pub const ALL: [PaperSize; 6] = [
        Self::Full,
        Self::A4,
        Self::Letter,
        Self::Legal,
        Self::A5,
        Self::Photo4x6,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Whole scan area",
            Self::A4 => "A4",
            Self::A5 => "A5",
            Self::Letter => "Letter",
            Self::Legal => "Legal",
            Self::Photo4x6 => "Photo, 4 x 6 in",
        }
    }

    /// (width, height) in 1/300 inch, or None for the full area.
    fn size(self) -> Option<(u32, u32)> {
        Some(match self {
            Self::Full => return None,
            Self::A4 => (2480, 3508),
            Self::A5 => (1748, 2480),
            Self::Letter => (2550, 3300),
            Self::Legal => (2550, 4200),
            Self::Photo4x6 => (1200, 1800),
        })
    }

    pub fn fits(self, caps: &InputCaps) -> bool {
        match self.size() {
            None => true,
            Some((w, h)) => w <= caps.max_width && h <= caps.max_height,
        }
    }

    pub fn region(self, caps: &InputCaps) -> Region {
        let (w, h) = self.size().unwrap_or((caps.max_width, caps.max_height));
        Region {
            x: 0,
            y: 0,
            width: w.clamp(caps.min_width, caps.max_width),
            height: h.clamp(caps.min_height, caps.max_height),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSettings {
    pub version: String,
    pub source: InputSource,
    pub duplex: bool,
    pub color_mode: ColorMode,
    pub resolution: u32,
    pub region: Region,
    pub document_format: String,
    /// Intent keyword, for example "Preview" or "Document" (eSCL section 7).
    pub intent: Option<String>,
    /// Whether to also send scan:DocumentFormatExt (eSCL 2.1+).
    pub send_format_ext: bool,
}

/// What the person asked for. Validated against the capabilities before use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRequest {
    pub source: InputSource,
    pub duplex: bool,
    pub color_mode: ColorMode,
    pub resolution: u32,
    pub paper: PaperSize,
    pub document_format: String,
    pub intent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    SourceNotSupported,
    ColorModeNotSupported,
    ResolutionNotSupported(u32),
    FormatNotSupported(String),
    PaperTooLarge,
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceNotSupported => write!(f, "the scanner does not have this input"),
            Self::ColorModeNotSupported => write!(f, "the scanner does not offer this color mode"),
            Self::ResolutionNotSupported(r) => write!(f, "the scanner does not offer {r} dpi"),
            Self::FormatNotSupported(m) => write!(f, "the scanner cannot produce {m}"),
            Self::PaperTooLarge => write!(f, "the paper size is larger than the scan area"),
        }
    }
}

impl std::error::Error for SettingsError {}

impl ScanSettings {
    /// Checks the request against the scanner and builds the settings, using
    /// the scanner's own eSCL version.
    pub fn for_request(
        caps: &ScannerCapabilities,
        req: &ScanRequest,
    ) -> Result<Self, SettingsError> {
        let input = caps
            .input(req.source, req.duplex)
            .ok_or(SettingsError::SourceNotSupported)?;
        if !input.color_modes.contains(&req.color_mode) {
            return Err(SettingsError::ColorModeNotSupported);
        }
        if !input.resolutions.contains(&req.resolution) {
            return Err(SettingsError::ResolutionNotSupported(req.resolution));
        }
        if !input.supports_format(&req.document_format) {
            return Err(SettingsError::FormatNotSupported(
                req.document_format.clone(),
            ));
        }
        if !req.paper.fits(input) {
            return Err(SettingsError::PaperTooLarge);
        }
        Ok(Self {
            version: caps.version.clone(),
            source: req.source,
            duplex: req.duplex,
            color_mode: req.color_mode.clone(),
            resolution: req.resolution,
            region: req.paper.region(input),
            document_format: req.document_format.clone(),
            intent: req.intent.clone(),
            send_format_ext: caps.version_tuple() >= (2, 1),
        })
    }

    pub fn to_xml(&self) -> String {
        let mut s = String::with_capacity(1024);
        s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        s.push_str(&format!(
            "<scan:ScanSettings xmlns:scan=\"{NS_SCAN}\" xmlns:pwg=\"{NS_PWG}\">\n"
        ));
        s.push_str(&format!(
            "  <pwg:Version>{}</pwg:Version>\n",
            escape(&self.version)
        ));
        if let Some(i) = &self.intent {
            s.push_str(&format!("  <scan:Intent>{}</scan:Intent>\n", escape(i)));
        }
        let r = self.region;
        s.push_str("  <pwg:ScanRegions>\n    <pwg:ScanRegion>\n");
        s.push_str(&format!("      <pwg:XOffset>{}</pwg:XOffset>\n", r.x));
        s.push_str(&format!("      <pwg:YOffset>{}</pwg:YOffset>\n", r.y));
        s.push_str(&format!("      <pwg:Width>{}</pwg:Width>\n", r.width));
        s.push_str(&format!("      <pwg:Height>{}</pwg:Height>\n", r.height));
        s.push_str(
            "      <pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits>\n",
        );
        s.push_str("    </pwg:ScanRegion>\n  </pwg:ScanRegions>\n");
        let fmt = escape(&self.document_format);
        s.push_str(&format!(
            "  <pwg:DocumentFormat>{fmt}</pwg:DocumentFormat>\n"
        ));
        if self.send_format_ext {
            s.push_str(&format!(
                "  <scan:DocumentFormatExt>{fmt}</scan:DocumentFormatExt>\n"
            ));
        }
        s.push_str(&format!(
            "  <pwg:InputSource>{}</pwg:InputSource>\n",
            self.source.keyword()
        ));
        s.push_str(&format!(
            "  <scan:XResolution>{}</scan:XResolution>\n",
            self.resolution
        ));
        s.push_str(&format!(
            "  <scan:YResolution>{}</scan:YResolution>\n",
            self.resolution
        ));
        s.push_str(&format!(
            "  <scan:ColorMode>{}</scan:ColorMode>\n",
            escape(self.color_mode.keyword())
        ));
        if self.source == InputSource::Feeder {
            s.push_str(&format!("  <scan:Duplex>{}</scan:Duplex>\n", self.duplex));
        }
        s.push_str("</scan:ScanSettings>\n");
        s
    }
}
