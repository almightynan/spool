// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Status of the printer that is on screen. One monitor runs at a time;
//! dropping its handle stops it.

use crate::cups::{CupsClient, CupsError};
use crate::ipp_client::{self, IppError, IppTarget};
use crate::net::{CancelToken, Limits};
use crate::poll::{spawn_poller, PollControl, PollerHandle};
use spool_protocols::ipp::{GroupTag, Operation, Request};
use spool_protocols::printer::{PrinterStatus, STATUS_ATTRIBUTES};
use std::fmt;
use std::sync::Arc;
use tokio::runtime::Handle;

/// Where the dashboard reads status from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusSource {
    /// Get-Printer-Attributes sent to the printer itself.
    Device { uri: String },
    /// Get-Printer-Attributes on the CUPS queue (for queues whose device is not
    /// an IPP printer, such as USB or cups-browsed queues).
    Queue { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusError {
    Ipp(IppError),
    Cups(CupsError),
    NoPrinterGroup,
}

impl fmt::Display for StatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ipp(e) => e.fmt(f),
            Self::Cups(e) => e.fmt(f),
            Self::NoPrinterGroup => write!(f, "the printer sent no printer attributes"),
        }
    }
}

pub async fn fetch_status(
    source: &StatusSource,
    cups: &CupsClient,
    cancel: &CancelToken,
) -> Result<PrinterStatus, StatusError> {
    match source {
        StatusSource::Device { uri } => {
            let target = IppTarget::for_uri(uri).map_err(|e| StatusError::Ipp(e.into()))?;
            let req = Request::new(Operation::GET_PRINTER_ATTRIBUTES)
                .printer_uri(uri)
                .user(cups.user())
                .requested_attributes(STATUS_ATTRIBUTES);
            let resp = ipp_client::send(&target, req, None, Limits::QUERY, cancel)
                .await
                .map_err(StatusError::Ipp)?;
            let g = resp
                .first_group(GroupTag::Printer)
                .ok_or(StatusError::NoPrinterGroup)?;
            Ok(PrinterStatus::from_attributes(g))
        }
        StatusSource::Queue { name } => cups
            .queue_status(name, cancel)
            .await
            .map_err(StatusError::Cups),
    }
}

impl StatusError {
    /// True when the printer did not answer at all, which the UI shows as offline.
    /// The network error behind an unreachable printer.
    pub fn net_error(&self) -> Option<&crate::net::NetError> {
        match self {
            Self::Ipp(IppError::Net(n)) => Some(n),
            _ => None,
        }
    }

    pub fn is_unreachable(&self) -> bool {
        match self {
            Self::Ipp(e) => e.is_unreachable(),
            Self::Cups(_) | Self::NoPrinterGroup => false,
        }
    }
}

pub type MonitorHandle = PollerHandle<Arc<PrinterStatus>, StatusError>;

/// Polls the printer's status on the shared schedule.
pub fn spawn_monitor(
    rt: &Handle,
    source: StatusSource,
    cups: Arc<CupsClient>,
    poll: &PollControl,
) -> MonitorHandle {
    spawn_poller(rt, poll, move |cancel| {
        let source = source.clone();
        let cups = cups.clone();
        async move { fetch_status(&source, &cups, &cancel).await.map(Arc::new) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poll::Polled;
    use crate::test_server::{fixture, serve_tcp, Canned, Routes};
    use std::path::PathBuf;
    use std::time::Duration;

    fn cups() -> Arc<CupsClient> {
        Arc::new(CupsClient::with_endpoints(
            "127.0.0.1".into(),
            9,
            PathBuf::from("/nonexistent"),
            "tester".into(),
        ))
    }

    #[tokio::test]
    async fn ready_then_unreachable() {
        let routes = Routes::default();
        routes.on(
            "POST /ipp/print",
            Canned::ok(fixture("printer-get-printer-attributes.ipp")),
        );
        routes.on(
            "POST /ipp/print",
            Canned::ok(vec![]).delayed(Duration::from_secs(30)),
        );
        let port = serve_tcp(routes).await;
        let poll = PollControl::new(Duration::from_millis(50));
        let mut h = spawn_monitor(
            &Handle::current(),
            StatusSource::Device {
                uri: format!("ipp://127.0.0.1:{port}/ipp/print"),
            },
            cups(),
            &poll,
        );
        h.state.changed().await.unwrap();
        let Polled::Ready(status) = h.state.borrow_and_update().clone() else {
            panic!("expected ready")
        };
        assert_eq!(status.markers.len(), 2);
        // The second poll hangs past the 10 s query timeout.
        tokio::time::timeout(Duration::from_secs(15), h.state.changed())
            .await
            .unwrap()
            .unwrap();
        let st = h.state.borrow().clone();
        assert!(
            matches!(&st, Polled::Failed(e) if e.is_unreachable()),
            "{st:?}"
        );
    }

    #[tokio::test]
    async fn refused_is_unreachable_and_cups_down_is_not() {
        let poll = PollControl::new(Duration::from_secs(60));
        let mut h = spawn_monitor(
            &Handle::current(),
            StatusSource::Device {
                uri: "ipp://127.0.0.1:9/ipp/print".into(),
            },
            cups(),
            &poll,
        );
        h.state.changed().await.unwrap();
        assert!(matches!(&*h.state.borrow(), Polled::Failed(e) if e.is_unreachable()));

        let mut h = spawn_monitor(
            &Handle::current(),
            StatusSource::Queue { name: "x".into() },
            cups(),
            &poll,
        );
        h.state.changed().await.unwrap();
        assert!(matches!(
            &*h.state.borrow(),
            Polled::Failed(StatusError::Cups(CupsError::NotRunning(_)))
        ));
    }
}
