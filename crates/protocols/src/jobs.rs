// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Print jobs from Get-Jobs responses (RFC 8011 section 4.2.6, 5.3).

use crate::ipp::{AttributeSet, Group, GroupTag, Response};

pub const JOB_ATTRIBUTES: &[&str] = &[
    "job-id",
    "job-name",
    "job-state",
    "job-state-reasons",
    "job-originating-user-name",
    "job-k-octets",
    "time-at-creation",
    "time-at-completed",
    "job-printer-uri",
    "job-impressions-completed",
];

/// job-state (RFC 8011 section 5.3.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Pending,
    Held,
    Processing,
    Stopped,
    Canceled,
    Aborted,
    Completed,
    Unknown(i32),
}

impl JobState {
    pub fn from_enum(v: i32) -> Self {
        match v {
            3 => Self::Pending,
            4 => Self::Held,
            5 => Self::Processing,
            6 => Self::Stopped,
            7 => Self::Canceled,
            8 => Self::Aborted,
            9 => Self::Completed,
            o => Self::Unknown(o),
        }
    }

    pub fn is_finished(self) -> bool {
        matches!(self, Self::Canceled | Self::Aborted | Self::Completed)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Waiting",
            Self::Held => "Held",
            Self::Processing => "Printing",
            Self::Stopped => "Stopped",
            Self::Canceled => "Canceled",
            Self::Aborted => "Failed",
            Self::Completed => "Completed",
            Self::Unknown(_) => "Unknown state",
        }
    }
}

/// Operations a job accepts in its current state (RFC 8011 sections 4.3.3,
/// 4.3.5, 4.3.6 and Table 7 for Restart-Job).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JobActions {
    pub cancel: bool,
    pub hold: bool,
    pub release: bool,
    pub retry: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: i32,
    pub name: String,
    pub state: JobState,
    pub reasons: Vec<String>,
    pub user: Option<String>,
    pub size_kb: Option<i32>,
    /// CUPS reports time-at-* as UNIX seconds (CUPS IPP spec).
    pub created: Option<i64>,
    pub completed: Option<i64>,
    pub printer_uri: Option<String>,
    pub pages_completed: Option<i32>,
}

impl Job {
    pub fn from_group(g: &Group) -> Option<Self> {
        let id = g.int_of("job-id")?;
        let name = g
            .str_of("job-name")
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Job {id}"));
        let time = |n: &str| g.int_of(n).filter(|t| *t > 0).map(i64::from);
        Some(Self {
            id,
            name,
            state: g
                .int_of("job-state")
                .map(JobState::from_enum)
                .unwrap_or(JobState::Unknown(0)),
            reasons: g
                .strs_of("job-state-reasons")
                .into_iter()
                .filter(|r| *r != "none")
                .map(str::to_string)
                .collect(),
            user: g.str_of("job-originating-user-name").map(str::to_string),
            size_kb: g.int_of("job-k-octets"),
            created: time("time-at-creation"),
            completed: time("time-at-completed"),
            printer_uri: g.str_of("job-printer-uri").map(str::to_string),
            pages_completed: g.int_of("job-impressions-completed").filter(|p| *p > 0),
        })
    }

    pub fn list_from(resp: &Response) -> Vec<Self> {
        resp.groups_of(GroupTag::Job)
            .filter_map(Self::from_group)
            .collect()
    }

    pub fn actions(&self) -> JobActions {
        use JobState::*;
        JobActions {
            cancel: matches!(self.state, Pending | Held | Processing | Stopped),
            hold: self.state == Pending,
            release: self.state == Held,
            retry: matches!(self.state, Completed | Canceled | Aborted),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(state: JobState) -> Job {
        Job {
            id: 1,
            name: "a".into(),
            state,
            reasons: vec![],
            user: None,
            size_kb: None,
            created: None,
            completed: None,
            printer_uri: None,
            pages_completed: None,
        }
    }

    #[test]
    fn actions_follow_rfc_8011_tables() {
        let a = job(JobState::Pending).actions();
        assert!(a.cancel && a.hold && !a.release && !a.retry);
        let a = job(JobState::Held).actions();
        assert!(a.cancel && !a.hold && a.release && !a.retry);
        let a = job(JobState::Processing).actions();
        assert!(a.cancel && !a.hold && !a.release && !a.retry);
        for s in [JobState::Completed, JobState::Canceled, JobState::Aborted] {
            let a = job(s).actions();
            assert!(!a.cancel && !a.hold && !a.release && a.retry);
        }
    }
}
