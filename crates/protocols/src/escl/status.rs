// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! ScannerStatus (Mopria eSCL Technical Specification section 9).

use super::xml::{Element, XmlError};

/// pwg:State values (eSCL section 9, PWG 5108.01 ScannerState).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScannerState {
    Idle,
    Processing,
    Testing,
    Stopped,
    Down,
    Other(String),
}

impl ScannerState {
    fn parse(s: &str) -> Self {
        match s {
            "Idle" => Self::Idle,
            "Processing" => Self::Processing,
            "Testing" => Self::Testing,
            "Stopped" => Self::Stopped,
            "Down" => Self::Down,
            o => Self::Other(o.to_string()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Idle => "Ready".into(),
            Self::Processing => "Busy scanning".into(),
            Self::Testing => "Running a self test".into(),
            Self::Stopped => "Stopped".into(),
            Self::Down => "Not working".into(),
            Self::Other(s) => s.clone(),
        }
    }

    pub fn can_scan(&self) -> bool {
        matches!(self, Self::Idle)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobInfo {
    pub uri: String,
    pub state: String,
    pub images_completed: Option<u32>,
    pub images_to_transfer: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannerStatus {
    pub state: ScannerState,
    /// scan:AdfState keyword, for example "ScannerAdfEmpty".
    pub adf_state: Option<String>,
    pub jobs: Vec<JobInfo>,
}

impl ScannerStatus {
    pub fn parse(xml: &str) -> Result<Self, XmlError> {
        let root = Element::parse(xml)?.expect_root("ScannerStatus")?;
        let state = root.child_text("State").ok_or(XmlError::Missing("State"))?;
        let jobs = root
            .child("Jobs")
            .into_iter()
            .flat_map(|j| j.children_named("JobInfo"))
            .filter_map(|j| {
                Some(JobInfo {
                    uri: j.child_text("JobUri")?.to_string(),
                    state: j.child_text("JobState").unwrap_or("").to_string(),
                    images_completed: j.child_u32("ImagesCompleted"),
                    images_to_transfer: j.child_u32("ImagesToTransfer"),
                })
            })
            .collect();
        Ok(Self {
            state: ScannerState::parse(state),
            adf_state: root.child_text("AdfState").map(str::to_string),
            jobs,
        })
    }

    /// True when the feeder reports paper loaded or is processing.
    pub fn adf_has_paper(&self) -> Option<bool> {
        match self.adf_state.as_deref()? {
            "ScannerAdfLoaded" | "ScannerAdfProcessing" => Some(true),
            "ScannerAdfEmpty" => Some(false),
            _ => None,
        }
    }

    /// A sentence for feeder problems (go-mfp adfstate.go lists the eSCL values).
    pub fn adf_sentence(&self) -> Option<&'static str> {
        Some(match self.adf_state.as_deref()? {
            "ScannerAdfEmpty" => "The document feeder is empty.",
            "ScannerAdfJam" => "Paper is jammed in the document feeder.",
            "ScannerAdfMispick" => "The document feeder could not pick up a page.",
            "ScannerAdfHatchOpen" => "The document feeder cover is open.",
            "ScannerAdfDuplexPageTooShort" => "A page is too short for two-sided scanning.",
            "ScannerAdfDuplexPageTooLong" => "A page is too long for two-sided scanning.",
            "ScannerAdfMultipickDetected" => "The document feeder picked up more than one page.",
            "ScannerAdfInputTrayFailed" => "The document feeder failed.",
            "ScannerAdfInputTrayOverloaded" => "The document feeder has too many pages.",
            _ => return None,
        })
    }
}
