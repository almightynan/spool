// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed network errors.

use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectFailure {
    Refused,
    Unreachable,
    /// Name did not resolve.
    UnknownHost,
    /// The local socket file (CUPS) does not exist.
    NoSocket,
    PermissionDenied,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    Cancelled,
    Timeout(Duration),
    Connect {
        target: String,
        failure: ConnectFailure,
    },
    Tls(String),
    /// HTTP framing or connection error after connecting.
    Protocol(String),
    BodyTooLarge(usize),
    InvalidRequest(String),
}

impl NetError {
    pub fn from_io(target: &str, e: &std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        let failure = match e.kind() {
            K::ConnectionRefused => ConnectFailure::Refused,
            K::HostUnreachable | K::NetworkUnreachable | K::AddrNotAvailable => {
                ConnectFailure::Unreachable
            }
            K::NotFound => ConnectFailure::NoSocket,
            K::PermissionDenied => ConnectFailure::PermissionDenied,
            _ => ConnectFailure::Other(e.to_string()),
        };
        Self::Connect {
            target: target.to_string(),
            failure,
        }
    }

    /// True when the device did not answer at all (used for "offline" states).
    pub fn is_unreachable(&self) -> bool {
        matches!(
            self,
            Self::Timeout(_)
                | Self::Connect {
                    failure: ConnectFailure::Refused
                        | ConnectFailure::Unreachable
                        | ConnectFailure::UnknownHost,
                    ..
                }
        )
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "canceled"),
            Self::Timeout(d) => write!(f, "no response within {} seconds", d.as_secs().max(1)),
            Self::Connect { target, failure } => match failure {
                ConnectFailure::Refused => write!(f, "{target} refused the connection"),
                ConnectFailure::Unreachable => write!(f, "{target} cannot be reached"),
                ConnectFailure::UnknownHost => write!(f, "{target} is not a known host name"),
                ConnectFailure::NoSocket => write!(f, "{target} does not exist"),
                ConnectFailure::PermissionDenied => write!(f, "no permission to open {target}"),
                ConnectFailure::Other(e) => write!(f, "cannot connect to {target}: {e}"),
            },
            Self::Tls(e) => write!(f, "secure connection failed: {e}"),
            Self::Protocol(e) => write!(f, "connection error: {e}"),
            Self::BodyTooLarge(n) => write!(f, "response is larger than {} MB", n / (1024 * 1024)),
            Self::InvalidRequest(e) => write!(f, "invalid request: {e}"),
        }
    }
}

impl std::error::Error for NetError {}
