// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! printer-state-reasons keywords (RFC 8011 section 5.4.12, PWG 5100.9 section 5.1)
//! translated to plain sentences.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Report,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateReason {
    /// Keyword without the severity suffix, for example "media-jam".
    pub keyword: String,
    pub severity: Severity,
}

impl StateReason {
    /// Parses one keyword. Returns None for "none" (RFC 8011: no reasons).
    ///
    /// When no suffix is present, RFC 8011 section 5.4.12 says to treat the reason
    /// as an error when the printer is stopped and a warning otherwise.
    pub fn parse(raw: &str, printer_stopped: bool) -> Option<Self> {
        if raw == "none" || raw.is_empty() {
            return None;
        }
        let (keyword, severity) = if let Some(k) = raw.strip_suffix("-error") {
            (k, Severity::Error)
        } else if let Some(k) = raw.strip_suffix("-warning") {
            (k, Severity::Warning)
        } else if let Some(k) = raw.strip_suffix("-report") {
            (k, Severity::Report)
        } else if printer_stopped {
            (raw, Severity::Error)
        } else {
            (raw, Severity::Warning)
        };
        Some(Self {
            keyword: keyword.to_string(),
            severity,
        })
    }

    /// Supply reasons are better described per marker; the status model drops them
    /// when a marker already explains the problem.
    pub fn is_supply_reason(&self) -> bool {
        matches!(
            self.keyword.as_str(),
            "marker-supply-low"
                | "marker-supply-empty"
                | "toner-low"
                | "toner-empty"
                | "marker-ink-almost-empty"
                | "marker-ink-empty"
                | "developer-low"
                | "developer-empty"
                | "marker-developer-almost-empty"
                | "marker-developer-empty"
        )
    }

    /// Reasons that carry no useful information for a person.
    pub fn is_noise(&self) -> bool {
        matches!(
            self.keyword.as_str(),
            "printer-ready-to-print"
                | "power-up"
                | "subunit-power-saver"
                | "cups-waiting-for-job-completed"
        )
    }

    /// Plain sentence for this reason.
    pub fn sentence(&self) -> String {
        match sentence_for(&self.keyword) {
            Some(s) => s.to_string(),
            None => format!("The printer reports \"{}\".", self.keyword),
        }
    }
}

fn sentence_for(keyword: &str) -> Option<&'static str> {
    Some(match keyword {
        // RFC 8011 section 5.4.12
        "other" => "The printer reports a problem it did not describe.",
        "connecting-to-device" => "Connecting to the printer.",
        "cover-open" => "A cover is open.",
        "developer-empty" => "The developer is empty.",
        "developer-low" => "The developer is low.",
        "door-open" => "A door is open.",
        "fuser-over-temp" => "The fuser is too hot.",
        "fuser-under-temp" => "The fuser is warming up.",
        "input-tray-missing" => "A paper tray is missing.",
        "interlock-open" => "An interlock is open.",
        "interpreter-resource-unavailable" => {
            "The printer ran out of memory or fonts for this job."
        }
        "marker-supply-empty" => "An ink or toner cartridge is empty.",
        "marker-supply-low" => "An ink or toner cartridge is low.",
        "marker-waste-almost-full" => "The waste container is almost full.",
        "marker-waste-full" => "The waste container is full.",
        "media-empty" => "The paper tray is empty.",
        "media-jam" => "Paper is jammed.",
        "media-low" => "Paper is running low.",
        "media-needed" => "Load paper to continue.",
        "moving-to-paused" => "The printer is pausing.",
        "opc-life-over" => "The photoconductor needs to be replaced.",
        "opc-near-eol" => "The photoconductor is almost worn out.",
        "output-area-almost-full" => "The output tray is almost full.",
        "output-area-full" => "The output tray is full. Remove printed pages.",
        "output-tray-missing" => "The output tray is missing.",
        "paused" => "The printer is paused.",
        "shutdown" => "The printer is turned off or shutting down.",
        "spool-area-full" => "The printer has no room for more jobs.",
        "stopped-partly" => "Part of the printer has stopped working.",
        "stopping" => "The printer is stopping.",
        "timed-out" => "The printer stopped responding.",
        "toner-empty" => "The toner is empty.",
        "toner-low" => "The toner is low.",
        // PWG 5100.9 section 5.1, Table 5-2
        "alert-removal-of-binary-change-entry" => "The printer changed its configuration.",
        "configuration-change" => "The printer configuration changed.",
        "cover-closed" => "A cover was closed.",
        "input-cannot-feed-size-selected" => "The tray cannot feed the selected paper size.",
        "input-manual-input-request" => "Insert paper into the manual feed slot.",
        "input-media-color-change" => "Change the paper color in the tray.",
        "input-media-form-parts-change" => "Change the paper form in the tray.",
        "input-media-size-change" => "Change the paper size in the tray.",
        "input-media-type-change" => "Change the paper type in the tray.",
        "input-media-weight-change" => "Change the paper weight in the tray.",
        "input-tray-elevation-failure" => "A paper tray cannot lift its paper.",
        "input-tray-position-failure" => "A paper tray is not seated correctly.",
        "interlock-closed" => "An interlock was closed.",
        "marker-adjusting-print-quality" => "The printer is adjusting print quality.",
        "marker-developer-almost-empty" => "The developer is almost empty.",
        "marker-developer-empty" => "The developer is empty.",
        "marker-fuser-thermistor-failure" => "The fuser temperature sensor failed.",
        "marker-fuser-timing-failure" => "The fuser has a timing failure.",
        "marker-ink-almost-empty" => "Ink is almost empty.",
        "marker-ink-empty" => "Ink is empty.",
        "marker-print-ribbon-almost-empty" => "The print ribbon is almost used up.",
        "marker-print-ribbon-empty" => "The print ribbon is used up.",
        "marker-toner-cartridge-missing" => "A toner cartridge is missing.",
        "marker-waste-ink-receptacle-full" => "The waste ink container is full.",
        "media-path-media-tray-almost-full" => "The output tray is almost full.",
        "media-path-media-tray-full" => "The output tray is full.",
        "media-path-media-tray-missing" => "The output tray is missing.",
        "output-mailbox-select-failure" => "The printer cannot select the output bin.",
        "power-down" => "The printer is turning off.",
        "printer-manual-reset" => "The printer was reset from its control panel.",
        "printer-nms-reset" => "The printer was reset over the network.",
        "subunit-almost-empty" => "A printer part is almost empty.",
        "subunit-almost-full" => "A printer part is almost full.",
        "subunit-empty" => "A printer part is empty.",
        "subunit-full" => "A printer part is full.",
        "subunit-life-almost-over" => "A printer part is almost worn out.",
        "subunit-life-over" => "A printer part needs to be replaced.",
        "subunit-missing" => "A printer part is missing.",
        "subunit-motor-failure" => "A printer motor failed.",
        "subunit-offline" => "A printer part is offline.",
        "subunit-opened" => "A printer part is open.",
        "subunit-over-temperature" => "A printer part is too hot.",
        "subunit-under-temperature" => "A printer part is too cold.",
        "subunit-unrecoverable-failure" => "A printer part failed and needs service.",
        "subunit-recoverable-failure" => "A printer part had a failure and recovered.",
        "subunit-warming-up" => "The printer is warming up.",
        // CUPS extensions (cups.org/doc/spec-ipp.html)
        "offline" => "The printer is not responding.",
        "cups-missing-filter" => "CUPS is missing a filter needed for this printer.",
        "cups-insecure-filter" => "CUPS refused a filter with unsafe permissions.",
        "cups-ipp-conformance-failure" => "The printer does not follow the IPP standard correctly.",
        "hold-new-jobs" => "New jobs are held.",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffixes() {
        let r = StateReason::parse("media-jam-error", false).unwrap();
        assert_eq!(r.keyword, "media-jam");
        assert_eq!(r.severity, Severity::Error);
        assert_eq!(r.sentence(), "Paper is jammed.");
        let r = StateReason::parse("toner-low-report", true).unwrap();
        assert_eq!(r.severity, Severity::Report);
    }

    #[test]
    fn missing_suffix_depends_on_state() {
        assert_eq!(
            StateReason::parse("cover-open", true).unwrap().severity,
            Severity::Error
        );
        assert_eq!(
            StateReason::parse("cover-open", false).unwrap().severity,
            Severity::Warning
        );
    }

    #[test]
    fn none_is_no_reason() {
        assert!(StateReason::parse("none", false).is_none());
    }

    #[test]
    fn unknown_keyword_is_quoted() {
        let r = StateReason::parse("com.example-thing-warning", false).unwrap();
        assert_eq!(r.sentence(), "The printer reports \"com.example-thing\".");
    }

    #[test]
    fn sentences_have_no_em_dash() {
        for k in ["media-jam", "toner-low", "output-area-full", "offline"] {
            assert!(!sentence_for(k).unwrap().contains('\u{2014}'));
        }
    }
}
