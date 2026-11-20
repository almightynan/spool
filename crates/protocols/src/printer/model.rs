// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Printer status built from Get-Printer-Attributes responses.

use super::media::{media_keyword_name, media_size_name};
use super::reasons::{Severity, StateReason};
use crate::ipp::{Attribute, AttributeSet, Value};

/// Attributes requested for the dashboard. Asking for exactly these keeps
/// responses small (the "all" group is 10 to 20 KB on typical printers).
pub const STATUS_ATTRIBUTES: &[&str] = &[
    "printer-name",
    "printer-info",
    "printer-location",
    "printer-make-and-model",
    "printer-state",
    "printer-state-reasons",
    "printer-state-message",
    "printer-is-accepting-jobs",
    "printer-uuid",
    "printer-more-info",
    "printer-firmware-string-version",
    "printer-dns-sd-name",
    "printer-uri-supported",
    "marker-names",
    "marker-colors",
    "marker-levels",
    "marker-low-levels",
    "marker-high-levels",
    "marker-types",
    "printer-input-tray",
    "media-col-ready",
    "media-ready",
];

/// printer-state (RFC 8011 section 5.4.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrinterState {
    Idle,
    Processing,
    Stopped,
    Unknown(i32),
}

impl PrinterState {
    pub fn from_enum(v: i32) -> Self {
        match v {
            3 => Self::Idle,
            4 => Self::Processing,
            5 => Self::Stopped,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    fn parse_list(s: &str) -> Vec<Rgb> {
        // CUPS spec: 'none' or one or more '#RRGGBB'.
        s.split('#')
            .filter(|p| p.len() == 6)
            .filter_map(|p| {
                let v = u32::from_str_radix(p, 16).ok()?;
                Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
            })
            .collect()
    }

    /// Plain name for the common cartridge colors, so a warning can say "cyan".
    pub fn common_name(self) -> Option<&'static str> {
        Some(match (self.0, self.1, self.2) {
            (0, 0, 0) => "black",
            (0x00, 0xFF, 0xFF) => "cyan",
            (0xFF, 0x00, 0xFF) => "magenta",
            (0xFF, 0xFF, 0x00) => "yellow",
            (0xFF, 0xFF, 0xFF) => "white",
            (0x80, 0x80, 0x80) | (0x7F, 0x7F, 0x7F) => "gray",
            _ => return None,
        })
    }
}

/// marker-levels value (CUPS IPP spec: -1 unavailable, -2 unknown,
/// -3 unknown but not empty, 0 to 100 percent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Percent(u8),
    Unavailable,
    Unknown,
    SomeRemaining,
}

impl Level {
    fn from_int(v: i32) -> Self {
        match v {
            0..=100 => Self::Percent(v as u8),
            -1 => Self::Unavailable,
            -3 => Self::SomeRemaining,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    Ink,
    Toner,
    Waste,
    Other,
}

impl MarkerKind {
    /// marker-types uses CUPS keywords ("ink-cartridge") or PWG supply types
    /// ("inkCartridge", PWG 5100.13 printer-supply). Both normalize the same way.
    fn from_keyword(k: &str) -> Self {
        let n: String = k
            .chars()
            .filter(|c| *c != '-')
            .flat_map(char::to_lowercase)
            .collect();
        if n.starts_with("waste") {
            Self::Waste
        } else if n.starts_with("toner") {
            Self::Toner
        } else if n.starts_with("ink") {
            Self::Ink
        } else {
            Self::Other
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Self::Ink => "Ink",
            Self::Toner => "Toner",
            Self::Waste => "Waste container",
            Self::Other => "Supply",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplyWarning {
    Low,
    Empty,
    /// Waste containers fill up instead of running out.
    AlmostFull,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub name: String,
    pub colors: Vec<Rgb>,
    pub level: Level,
    pub low_level: Option<u8>,
    pub high_level: Option<u8>,
    pub kind: MarkerKind,
    pub warning: Option<SupplyWarning>,
}

impl Marker {
    /// Short label used inside warnings: a color name when the marker has one
    /// known color, otherwise the printer's marker name.
    pub fn label(&self) -> String {
        match self.colors.as_slice() {
            [c] => c
                .common_name()
                .map(str::to_string)
                .unwrap_or_else(|| self.name.clone()),
            _ => self.name.clone(),
        }
    }

    /// For example "Ink is low (cyan 12%)".
    pub fn warning_sentence(&self) -> Option<String> {
        let w = self.warning?;
        let noun = self.kind.noun();
        let state = match w {
            SupplyWarning::Low => "is low",
            SupplyWarning::Empty => "is empty",
            SupplyWarning::AlmostFull => "is almost full",
            SupplyWarning::Full => "is full",
        };
        let detail = match self.level {
            Level::Percent(p) => format!("{} {p}%", self.label()),
            _ => self.label(),
        };
        Some(format!("{noun} {state} ({detail})"))
    }

    fn compute_warning(&self) -> Option<SupplyWarning> {
        let Level::Percent(p) = self.level else {
            return None;
        };
        if self.kind == MarkerKind::Waste {
            // High level marks near-full for waste (CUPS spec marker-high-levels).
            return match self.high_level {
                _ if p >= 100 => Some(SupplyWarning::Full),
                Some(h) if h < 100 && p >= h => Some(SupplyWarning::AlmostFull),
                _ => None,
            };
        }
        if p == 0 {
            return Some(SupplyWarning::Empty);
        }
        match self.low_level {
            Some(low) if p <= low => Some(SupplyWarning::Low),
            _ => None,
        }
    }
}

/// Subunit status bits (RFC 3805 PrtSubUnitStatusTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SubunitStatus {
    pub unavailable: bool,
    pub broken: bool,
    pub non_critical_alert: bool,
    pub critical_alert: bool,
    pub offline: bool,
}

impl SubunitStatus {
    fn from_bits(v: i32) -> Self {
        let availability = v & 0x7;
        Self {
            unavailable: matches!(availability, 1 | 3),
            broken: availability == 3,
            non_critical_alert: v & 8 != 0,
            critical_alert: v & 16 != 0,
            offline: v & 32 != 0,
        }
    }
}

/// Tray fill level (RFC 3805 prtInputCurrentLevel: -1 other, -2 unknown,
/// -3 at least one sheet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayLevel {
    Sheets { current: u32, max: Option<u32> },
    Empty,
    NotEmpty,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tray {
    pub name: String,
    pub media: Option<String>,
    pub media_type: Option<String>,
    pub level: TrayLevel,
    pub status: SubunitStatus,
}

impl Tray {
    /// Parses one printer-input-tray value (PWG 5100.13 section 5.6.32.2:
    /// "key=value;" pairs). Returns None for values without a name or type.
    pub fn parse(raw: &str) -> Option<Self> {
        let mut name = None;
        let mut level = None;
        let mut max = None;
        let mut status = 0;
        let mut media = None;
        let mut media_type = None;
        let mut kind = None;
        for pair in raw.split(';') {
            let Some((k, v)) = pair.split_once('=') else {
                continue;
            };
            match k.trim() {
                "name" => name = Some(v.to_string()),
                "type" => kind = Some(v.to_string()),
                "level" => level = v.parse::<i32>().ok(),
                "maxcapacity" => max = v.parse::<i32>().ok(),
                "status" => status = v.parse::<i32>().unwrap_or(0),
                "medianame" if !v.is_empty() => media = Some(v.to_string()),
                "mediatype" if !v.is_empty() => media_type = Some(v.to_string()),
                _ => {}
            }
        }
        let name = match (name, kind) {
            (Some(n), _) if !n.is_empty() => n,
            (_, Some(k)) if !k.is_empty() => k,
            _ => return None,
        };
        let max = max.filter(|m| *m > 0).map(|m| m as u32);
        let level = match level {
            Some(0) => TrayLevel::Empty,
            Some(l) if l > 0 => TrayLevel::Sheets {
                current: l as u32,
                max,
            },
            Some(-3) => TrayLevel::NotEmpty,
            _ => TrayLevel::Unknown,
        };
        Some(Self {
            name,
            media,
            media_type,
            level,
            status: SubunitStatus::from_bits(status),
        })
    }
}

/// One entry of media-col-ready (PWG 5100.7 / 5100.13) or media-ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaReady {
    pub source: Option<String>,
    pub size: String,
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrinterInfo {
    pub name: Option<String>,
    pub info: Option<String>,
    pub location: Option<String>,
    pub make_and_model: Option<String>,
    pub uuid: Option<String>,
    pub more_info: Option<String>,
    pub firmware: Option<String>,
    pub dns_sd_name: Option<String>,
    pub uris: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrinterStatus {
    pub state: PrinterState,
    pub reasons: Vec<StateReason>,
    pub state_message: Option<String>,
    pub accepting_jobs: Option<bool>,
    pub markers: Vec<Marker>,
    pub trays: Vec<Tray>,
    pub media_ready: Vec<MediaReady>,
    pub info: PrinterInfo,
}

/// A plain sentence about the printer with a severity, ready for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub severity: Severity,
    pub text: String,
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl PrinterStatus {
    /// Builds the status from a printer attributes group.
    pub fn from_attributes<A: AttributeSet + ?Sized>(attrs: &A) -> Self {
        let state = attrs
            .int_of("printer-state")
            .map(PrinterState::from_enum)
            .unwrap_or(PrinterState::Unknown(0));
        let stopped = state == PrinterState::Stopped;
        let reasons = attrs
            .strs_of("printer-state-reasons")
            .into_iter()
            .filter_map(|r| StateReason::parse(r, stopped))
            .collect();

        let names = attrs.strs_of("marker-names");
        let colors = attrs.strs_of("marker-colors");
        let levels = attrs.ints_of("marker-levels");
        let lows = attrs.ints_of("marker-low-levels");
        let highs = attrs.ints_of("marker-high-levels");
        let types = attrs.strs_of("marker-types");
        let pct = |v: Option<&i32>| v.and_then(|v| u8::try_from(*v).ok()).filter(|v| *v <= 100);
        let markers = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let mut m = Marker {
                    name: name.to_string(),
                    colors: colors
                        .get(i)
                        .map(|c| Rgb::parse_list(c))
                        .unwrap_or_default(),
                    level: levels
                        .get(i)
                        .map(|v| Level::from_int(*v))
                        .unwrap_or(Level::Unknown),
                    low_level: pct(lows.get(i)),
                    high_level: pct(highs.get(i)),
                    kind: types
                        .get(i)
                        .map(|t| MarkerKind::from_keyword(t))
                        .unwrap_or(MarkerKind::Other),
                    warning: None,
                };
                m.warning = m.compute_warning();
                m
            })
            .collect();

        let trays = attrs
            .strs_of("printer-input-tray")
            .into_iter()
            .filter_map(Tray::parse)
            .collect();

        let mut media_ready: Vec<MediaReady> = attrs
            .get("media-col-ready")
            .map(|a| a.values.iter().filter_map(media_col_entry).collect())
            .unwrap_or_default();
        if media_ready.is_empty() {
            media_ready = attrs
                .strs_of("media-ready")
                .into_iter()
                .map(|k| MediaReady {
                    source: None,
                    size: media_keyword_name(k),
                    media_type: None,
                })
                .collect();
        }

        let info = PrinterInfo {
            name: non_empty(attrs.str_of("printer-name")),
            info: non_empty(attrs.str_of("printer-info")),
            location: non_empty(attrs.str_of("printer-location")),
            make_and_model: non_empty(attrs.str_of("printer-make-and-model")),
            uuid: non_empty(attrs.str_of("printer-uuid")),
            more_info: non_empty(attrs.str_of("printer-more-info")),
            firmware: non_empty(attrs.str_of("printer-firmware-string-version")),
            dns_sd_name: non_empty(attrs.str_of("printer-dns-sd-name")),
            uris: attrs
                .strs_of("printer-uri-supported")
                .into_iter()
                .map(str::to_string)
                .collect(),
        };

        Self {
            state,
            reasons,
            state_message: non_empty(attrs.str_of("printer-state-message")),
            accepting_jobs: attrs.bool_of("printer-is-accepting-jobs"),
            markers,
            trays,
            media_ready,
            info,
        }
    }

    /// Sentences to show on the dashboard, most severe first. Supply reasons are
    /// replaced by per-marker sentences when a marker explains them.
    pub fn notices(&self) -> Vec<Notice> {
        let mut out: Vec<Notice> = Vec::new();
        let marker_notices: Vec<Notice> = self
            .markers
            .iter()
            .filter_map(|m| {
                let text = m.warning_sentence()?;
                let severity = match m.warning {
                    Some(SupplyWarning::Empty) | Some(SupplyWarning::Full) => Severity::Error,
                    _ => Severity::Warning,
                };
                Some(Notice { severity, text })
            })
            .collect();
        let have_marker_notice = !marker_notices.is_empty();
        out.extend(marker_notices);
        for r in &self.reasons {
            if r.is_noise() || (have_marker_notice && r.is_supply_reason()) {
                continue;
            }
            out.push(Notice {
                severity: r.severity,
                text: r.sentence(),
            });
        }
        // Different keywords can share a sentence (for example two "output tray
        // is full" reasons); keep the first, most severe one.
        out.sort_by_key(|n| std::cmp::Reverse(n.severity));
        let mut seen = std::collections::HashSet::new();
        out.retain(|n| seen.insert(n.text.clone()));
        out
    }
}

fn media_col_entry(v: &Value) -> Option<MediaReady> {
    let col: &[Attribute] = v.as_collection()?;
    let size = col.collection_of("media-size").and_then(|s| {
        let x = s.int_of("x-dimension")?;
        let y = s.int_of("y-dimension")?;
        Some(media_size_name(x, y))
    });
    let size = size.or_else(|| col.str_of("media-size-name").map(media_keyword_name))?;
    Some(MediaReady {
        source: non_empty(col.str_of("media-source")),
        size,
        media_type: non_empty(col.str_of("media-type")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(level: i32, low: Option<u8>, kind: MarkerKind, color: &str) -> Marker {
        let mut m = Marker {
            name: "x".into(),
            colors: Rgb::parse_list(color),
            level: Level::from_int(level),
            low_level: low,
            high_level: Some(100),
            kind,
            warning: None,
        };
        m.warning = m.compute_warning();
        m
    }

    #[test]
    fn low_ink_sentence_uses_color_name() {
        let m = marker(12, Some(15), MarkerKind::Ink, "#00FFFF");
        assert_eq!(m.warning_sentence().unwrap(), "Ink is low (cyan 12%)");
    }

    #[test]
    fn multi_color_marker_uses_its_name() {
        let mut m = marker(3, Some(5), MarkerKind::Ink, "#00FFFF#FF00FF#FFFF00");
        m.name = "tri-color ink".into();
        assert_eq!(
            m.warning_sentence().unwrap(),
            "Ink is low (tri-color ink 3%)"
        );
    }

    #[test]
    fn empty_and_unknown_levels() {
        assert_eq!(
            marker(0, Some(5), MarkerKind::Toner, "#000000").warning,
            Some(SupplyWarning::Empty)
        );
        assert_eq!(
            marker(-2, Some(5), MarkerKind::Ink, "#000000").warning,
            None
        );
        assert_eq!(Level::from_int(-3), Level::SomeRemaining);
    }

    #[test]
    fn no_low_threshold_means_no_low_warning() {
        assert_eq!(marker(4, None, MarkerKind::Ink, "#000000").warning, None);
    }

    #[test]
    fn waste_warns_near_full() {
        let mut m = marker(95, None, MarkerKind::Waste, "none");
        m.high_level = Some(90);
        assert_eq!(m.compute_warning(), Some(SupplyWarning::AlmostFull));
    }

    #[test]
    fn marker_kind_accepts_cups_and_pwg_keywords() {
        assert_eq!(MarkerKind::from_keyword("ink-cartridge"), MarkerKind::Ink);
        assert_eq!(MarkerKind::from_keyword("inkCartridge"), MarkerKind::Ink);
        assert_eq!(
            MarkerKind::from_keyword("toner-cartridge"),
            MarkerKind::Toner
        );
        assert_eq!(MarkerKind::from_keyword("wasteInk"), MarkerKind::Waste);
    }

    #[test]
    fn tray_parsing() {
        let t = Tray::parse(
            "type=sheetFeedAutoRemovableTray;mediafeed=0;mediaxfeed=0;maxcapacity=250;level=-3;status=0;name=Tray 2;",
        )
        .unwrap();
        assert_eq!(t.name, "Tray 2");
        assert_eq!(t.level, TrayLevel::NotEmpty);
        let t = Tray::parse("type=sheetFeedManual;maxcapacity=100;level=40;status=40;name=Manual;")
            .unwrap();
        assert_eq!(
            t.level,
            TrayLevel::Sheets {
                current: 40,
                max: Some(100)
            }
        );
        assert!(t.status.offline);
        assert!(t.status.non_critical_alert);
        assert!(Tray::parse("level=3;").is_none());
    }
}
