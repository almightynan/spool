// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! ScannerCapabilities (Mopria eSCL Technical Specification section 8.1).

use super::xml::{Element, XmlError};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ColorMode {
    BlackAndWhite1,
    Grayscale8,
    Grayscale16,
    Rgb24,
    Rgb48,
    Other(String),
}

impl ColorMode {
    pub fn parse(s: &str) -> Self {
        match s {
            "BlackAndWhite1" => Self::BlackAndWhite1,
            "Grayscale8" => Self::Grayscale8,
            "Grayscale16" => Self::Grayscale16,
            "RGB24" => Self::Rgb24,
            "RGB48" => Self::Rgb48,
            o => Self::Other(o.to_string()),
        }
    }

    pub fn keyword(&self) -> &str {
        match self {
            Self::BlackAndWhite1 => "BlackAndWhite1",
            Self::Grayscale8 => "Grayscale8",
            Self::Grayscale16 => "Grayscale16",
            Self::Rgb24 => "RGB24",
            Self::Rgb48 => "RGB48",
            Self::Other(s) => s,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::BlackAndWhite1 => "Black and white".into(),
            Self::Grayscale8 => "Grayscale".into(),
            Self::Grayscale16 => "Grayscale, 16 bit".into(),
            Self::Rgb24 => "Color".into(),
            Self::Rgb48 => "Color, 16 bit".into(),
            Self::Other(s) => s.clone(),
        }
    }

    /// Ordering for menus: color first, as most scans are color.
    fn rank(&self) -> u8 {
        match self {
            Self::Rgb24 => 0,
            Self::Grayscale8 => 1,
            Self::BlackAndWhite1 => 2,
            Self::Rgb48 => 3,
            Self::Grayscale16 => 4,
            Self::Other(_) => 5,
        }
    }
}

/// Input source (eSCL section 7, pwg:InputSource).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputSource {
    Platen,
    Feeder,
}

impl InputSource {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Platen => "Platen",
            Self::Feeder => "Feeder",
        }
    }
}

/// Capabilities of one input (platen, ADF simplex or ADF duplex), with all
/// setting profiles merged. Dimensions are in 1/300 inch (eSCL ContentRegionUnits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputCaps {
    pub min_width: u32,
    pub max_width: u32,
    pub min_height: u32,
    pub max_height: u32,
    pub color_modes: Vec<ColorMode>,
    pub document_formats: Vec<String>,
    /// Resolutions with equal X and Y, ascending.
    pub resolutions: Vec<u32>,
    pub intents: Vec<String>,
}

/// Resolutions offered when a scanner gives a range instead of a list.
const COMMON_RESOLUTIONS: &[u32] = &[75, 100, 150, 200, 300, 600, 1200, 2400];

impl InputCaps {
    fn parse(e: &Element) -> Option<Self> {
        let mut caps = Self {
            min_width: e.child_u32("MinWidth")?,
            max_width: e.child_u32("MaxWidth")?,
            min_height: e.child_u32("MinHeight")?,
            max_height: e.child_u32("MaxHeight")?,
            color_modes: Vec::new(),
            document_formats: Vec::new(),
            resolutions: Vec::new(),
            intents: e
                .child("SupportedIntents")
                .map(|i| {
                    i.children_named("Intent")
                        .map(|c| c.text.trim().to_string())
                        .collect()
                })
                .unwrap_or_default(),
        };
        let profiles = e.child("SettingProfiles");
        for p in profiles
            .into_iter()
            .flat_map(|p| p.children_named("SettingProfile"))
        {
            for m in p
                .child("ColorModes")
                .into_iter()
                .flat_map(|c| c.children_named("ColorMode"))
            {
                let m = ColorMode::parse(m.text.trim());
                if !caps.color_modes.contains(&m) {
                    caps.color_modes.push(m);
                }
            }
            if let Some(f) = p.child("DocumentFormats") {
                for d in f
                    .children
                    .iter()
                    .filter(|c| c.name == "DocumentFormat" || c.name == "DocumentFormatExt")
                {
                    let d = d.text.trim().to_string();
                    if !d.is_empty() && !caps.document_formats.contains(&d) {
                        caps.document_formats.push(d);
                    }
                }
            }
            if let Some(r) = p.child("SupportedResolutions") {
                for d in r
                    .child("DiscreteResolutions")
                    .into_iter()
                    .flat_map(|d| d.children_named("DiscreteResolution"))
                {
                    if let (Some(x), Some(y)) =
                        (d.child_u32("XResolution"), d.child_u32("YResolution"))
                    {
                        if x == y {
                            caps.resolutions.push(x);
                        }
                    }
                }
                if let Some(range) = r.child("ResolutionRange") {
                    let bounds = |n: &str| {
                        let rr = range.child(n)?;
                        Some((rr.child_u32("Min")?, rr.child_u32("Max")?))
                    };
                    if let (Some((xmin, xmax)), Some((ymin, ymax))) =
                        (bounds("XResolutionRange"), bounds("YResolutionRange"))
                    {
                        let (lo, hi) = (xmin.max(ymin), xmax.min(ymax));
                        caps.resolutions.extend(
                            COMMON_RESOLUTIONS
                                .iter()
                                .copied()
                                .filter(|v| *v >= lo && *v <= hi),
                        );
                    }
                }
            }
        }
        caps.color_modes.sort_by_key(ColorMode::rank);
        caps.resolutions.sort_unstable();
        caps.resolutions.dedup();
        Some(caps)
    }

    pub fn supports_format(&self, mime: &str) -> bool {
        self.document_formats.iter().any(|f| f == mime)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannerCapabilities {
    /// pwg:Version; echoed in ScanSettings so the request matches what the
    /// scanner says it implements.
    pub version: String,
    pub make_and_model: Option<String>,
    pub uuid: Option<String>,
    pub admin_uri: Option<String>,
    pub platen: Option<InputCaps>,
    pub adf_simplex: Option<InputCaps>,
    pub adf_duplex: Option<InputCaps>,
    pub feeder_capacity: Option<u32>,
}

impl ScannerCapabilities {
    pub fn parse(xml: &str) -> Result<Self, XmlError> {
        let root = Element::parse(xml)?.expect_root("ScannerCapabilities")?;
        let version = root
            .child_text("Version")
            .ok_or(XmlError::Missing("Version"))?
            .to_string();
        let adf = root.child("Adf");
        Ok(Self {
            version,
            make_and_model: root.child_text("MakeAndModel").map(str::to_string),
            uuid: root.child_text("UUID").map(str::to_string),
            admin_uri: root.child_text("AdminURI").map(str::to_string),
            platen: root
                .path(&["Platen", "PlatenInputCaps"])
                .and_then(InputCaps::parse),
            adf_simplex: adf
                .and_then(|a| a.child("AdfSimplexInputCaps"))
                .and_then(InputCaps::parse),
            adf_duplex: adf
                .and_then(|a| a.child("AdfDuplexInputCaps"))
                .and_then(InputCaps::parse),
            feeder_capacity: adf.and_then(|a| a.child_u32("FeederCapacity")),
        })
    }

    /// Input caps for a source; duplex uses the ADF duplex caps.
    pub fn input(&self, source: InputSource, duplex: bool) -> Option<&InputCaps> {
        match (source, duplex) {
            (InputSource::Platen, _) => self.platen.as_ref(),
            (InputSource::Feeder, false) => self.adf_simplex.as_ref(),
            (InputSource::Feeder, true) => self.adf_duplex.as_ref(),
        }
    }

    pub fn sources(&self) -> Vec<InputSource> {
        let mut v = Vec::new();
        if self.platen.is_some() {
            v.push(InputSource::Platen);
        }
        if self.adf_simplex.is_some() || self.adf_duplex.is_some() {
            v.push(InputSource::Feeder);
        }
        v
    }

    /// eSCL version as (major, minor), used to decide on DocumentFormatExt (2.1+).
    pub fn version_tuple(&self) -> (u32, u32) {
        let mut it = self
            .version
            .split('.')
            .map(|p| p.parse::<u32>().unwrap_or(0));
        (it.next().unwrap_or(0), it.next().unwrap_or(0))
    }
}
