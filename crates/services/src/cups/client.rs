// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! CUPS over IPP (CUPS IPP spec, RFC 8011).
//!
//! Queries go to localhost:631. Operations that change state go over the
//! CUPS domain socket with `Authorization: PeerCred <user>`, because cupsd
//! accepts peer credentials only on AF_LOCAL connections (cups 2.4
//! scheduler/auth.c, cupsdAuthorize). This lets members of the CUPS system
//! group (usually lpadmin) add and remove queues without a password.

use super::testpage::{TEST_PAGE, TEST_PAGE_FORMAT};
use crate::ipp_client::{self, IppError, IppTarget};
use crate::net::{CancelToken, ConnectFailure, Limits, NetError, Target};
use spool_protocols::cups::{Queue, QUEUE_ATTRIBUTES};
use spool_protocols::ipp::{
    Attribute, AttributeSet, GroupTag, Operation, Request, Response, StatusCode, Value,
};
use spool_protocols::jobs::{Job, JOB_ATTRIBUTES};
use spool_protocols::printer::{PrinterStatus, STATUS_ATTRIBUTES};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_SOCKET: &str = "/run/cups/cups.sock";

/// cupsd generates the IPP Everywhere PPD by querying the printer before it
/// answers CUPS-Add-Modify-Printer, which can take a while.
const ADD_QUEUE_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CupsError {
    /// cupsd did not answer on localhost:631 or the socket.
    NotRunning(NetError),
    /// The account may not perform this operation (HTTP 401/403 or IPP
    /// not-authorized/forbidden).
    NotAllowed,
    NotFound,
    /// Restart-Job refused because the job files are gone.
    NotPossible(Option<String>),
    /// The queue cannot print the CUPS test page banner.
    TestPageUnavailable,
    Other(IppError),
}

impl From<IppError> for CupsError {
    fn from(e: IppError) -> Self {
        match &e {
            IppError::Net(n) if is_down(n) => Self::NotRunning(n.clone()),
            IppError::Http(401) | IppError::Http(403) => Self::NotAllowed,
            IppError::Status { status, message } => match *status {
                StatusCode::NOT_AUTHORIZED
                | StatusCode::FORBIDDEN
                | StatusCode::NOT_AUTHENTICATED => Self::NotAllowed,
                StatusCode::NOT_FOUND => Self::NotFound,
                StatusCode::NOT_POSSIBLE => Self::NotPossible(message.clone()),
                _ => Self::Other(e),
            },
            _ => Self::Other(e),
        }
    }
}

fn is_down(n: &NetError) -> bool {
    matches!(
        n,
        NetError::Connect {
            failure: ConnectFailure::Refused
                | ConnectFailure::NoSocket
                | ConnectFailure::PermissionDenied,
            ..
        } | NetError::Timeout(_)
    )
}

impl fmt::Display for CupsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning(e) => write!(f, "the print service (CUPS) is not responding: {e}"),
            Self::NotAllowed => write!(f, "your account is not allowed to manage printers"),
            Self::NotFound => write!(f, "the print queue or job no longer exists"),
            Self::NotPossible(Some(m)) => write!(f, "{m}"),
            Self::NotPossible(None) => {
                write!(f, "CUPS cannot do this for the job in its current state")
            }
            Self::TestPageUnavailable => {
                write!(
                    f,
                    "this queue cannot print the CUPS test page (cups-filters is missing)"
                )
            }
            Self::Other(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for CupsError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhichJobs {
    NotCompleted,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOp {
    Cancel,
    Hold,
    Release,
    Restart,
}

impl JobOp {
    fn operation(self) -> Operation {
        match self {
            Self::Cancel => Operation::CANCEL_JOB,
            Self::Hold => Operation::HOLD_JOB,
            Self::Release => Operation::RELEASE_JOB,
            Self::Restart => Operation::RESTART_JOB,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewQueue {
    pub name: String,
    pub device_uri: String,
    pub info: String,
    pub location: String,
}

#[derive(Debug, Clone)]
pub struct CupsClient {
    host: String,
    port: u16,
    socket: PathBuf,
    user: String,
}

/// Percent-encodes a queue name for use in a URI path (RFC 3986 unreserved
/// characters stay as they are).
fn encode_name(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl CupsClient {
    pub fn new(user: String) -> Self {
        Self::with_endpoints("localhost".into(), 631, PathBuf::from(DEFAULT_SOCKET), user)
    }

    pub fn with_endpoints(host: String, port: u16, socket: PathBuf, user: String) -> Self {
        Self {
            host,
            port,
            socket,
            user,
        }
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    /// printer-uri CUPS uses for its own queues.
    pub fn queue_uri(name: &str) -> String {
        format!("ipp://localhost/printers/{}", encode_name(name))
    }

    fn read_target(&self, path: &str, printer_uri: String) -> IppTarget {
        IppTarget {
            target: Target::Tcp {
                host: self.host.clone(),
                port: self.port,
                tls: false,
            },
            host_header: format!("{}:{}", self.host, self.port),
            path: path.to_string(),
            printer_uri,
        }
    }

    fn write_target(&self, path: &str, printer_uri: String) -> IppTarget {
        IppTarget {
            target: Target::Unix(self.socket.clone()),
            host_header: "localhost".into(),
            path: path.to_string(),
            printer_uri,
        }
    }

    fn peercred(&self) -> String {
        format!("PeerCred {}", self.user)
    }

    async fn read(
        &self,
        path: &str,
        req: Request,
        cancel: &CancelToken,
    ) -> Result<Response, CupsError> {
        let t = self.read_target(path, String::new());
        Ok(ipp_client::send(&t, req.user(&self.user), None, Limits::QUERY, cancel).await?)
    }

    async fn write(
        &self,
        path: &str,
        req: Request,
        limits: Limits,
        cancel: &CancelToken,
    ) -> Result<Response, CupsError> {
        let t = self.write_target(path, String::new());
        let auth = self.peercred();
        Ok(ipp_client::send(&t, req.user(&self.user), Some(&auth), limits, cancel).await?)
    }

    pub async fn queues(&self, cancel: &CancelToken) -> Result<Vec<Queue>, CupsError> {
        let req = Request::new(Operation::CUPS_GET_PRINTERS).requested_attributes(QUEUE_ATTRIBUTES);
        let resp = match self.read("/", req, cancel).await {
            // CUPS answers client-error-not-found when there are no queues.
            Err(CupsError::NotFound) => return Ok(Vec::new()),
            r => r?,
        };
        Ok(Queue::list_from(&resp))
    }

    /// Name of the default queue, or None when no default is set.
    pub async fn default_queue(&self, cancel: &CancelToken) -> Result<Option<String>, CupsError> {
        let req = Request::new(Operation::CUPS_GET_DEFAULT).requested_attributes(&["printer-name"]);
        match self.read("/", req, cancel).await {
            Ok(resp) => Ok(resp
                .first_group(GroupTag::Printer)
                .and_then(|g| g.str_of("printer-name"))
                .map(str::to_string)),
            Err(CupsError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn queue_status(
        &self,
        name: &str,
        cancel: &CancelToken,
    ) -> Result<PrinterStatus, CupsError> {
        let uri = Self::queue_uri(name);
        let req = Request::new(Operation::GET_PRINTER_ATTRIBUTES)
            .printer_uri(&uri)
            .requested_attributes(STATUS_ATTRIBUTES);
        let resp = self
            .read(&format!("/printers/{}", encode_name(name)), req, cancel)
            .await?;
        let g = resp
            .first_group(GroupTag::Printer)
            .ok_or(CupsError::NotFound)?;
        Ok(PrinterStatus::from_attributes(g))
    }

    pub async fn jobs(
        &self,
        name: &str,
        which: WhichJobs,
        cancel: &CancelToken,
    ) -> Result<Vec<Job>, CupsError> {
        let uri = Self::queue_uri(name);
        let which = match which {
            WhichJobs::NotCompleted => "not-completed",
            WhichJobs::Completed => "completed",
        };
        let req = Request::new(Operation::GET_JOBS)
            .printer_uri(&uri)
            .op_attr(Attribute::new("which-jobs", Value::keyword(which)))
            .requested_attributes(JOB_ATTRIBUTES);
        let resp = self
            .read(&format!("/printers/{}", encode_name(name)), req, cancel)
            .await?;
        let mut jobs = Job::list_from(&resp);
        // Newest first.
        jobs.sort_by_key(|j| std::cmp::Reverse(j.id));
        Ok(jobs)
    }

    /// Cancel-Job, Hold-Job, Release-Job or Restart-Job (RFC 8011 4.3.3, 4.3.5
    /// to 4.3.7), addressed by printer-uri and job-id.
    pub async fn job_op(
        &self,
        queue: &str,
        job_id: i32,
        op: JobOp,
        cancel: &CancelToken,
    ) -> Result<(), CupsError> {
        let req = Request::new(op.operation())
            .printer_uri(&Self::queue_uri(queue))
            .op_attr(Attribute::new("job-id", Value::Integer(job_id)));
        self.write(
            &format!("/printers/{}", encode_name(queue)),
            req,
            Limits::QUERY,
            cancel,
        )
        .await?;
        Ok(())
    }

    /// CUPS-Set-Default (CUPS IPP spec, operation 0x400A).
    pub async fn set_default(&self, name: &str, cancel: &CancelToken) -> Result<(), CupsError> {
        let req = Request::new(Operation::CUPS_SET_DEFAULT).printer_uri(&Self::queue_uri(name));
        self.write("/admin/", req, Limits::QUERY, cancel).await?;
        Ok(())
    }

    /// CUPS-Add-Modify-Printer with the IPP Everywhere driver. cupsd builds the
    /// PPD itself for ipp, ipps and dnssd device URIs (cups 2.4 scheduler/ipp.c).
    pub async fn add_queue(&self, q: &NewQueue, cancel: &CancelToken) -> Result<(), CupsError> {
        let mut attrs = vec![
            Attribute::new("device-uri", Value::uri(&q.device_uri)),
            Attribute::new("ppd-name", Value::name("everywhere")),
            Attribute::new("printer-is-accepting-jobs", Value::Boolean(true)),
            // idle(3): the only enabled state cupsd recognizes here (CUPS spec).
            Attribute::new("printer-state", Value::Enum(3)),
        ];
        if !q.info.is_empty() {
            attrs.push(Attribute::new("printer-info", Value::text(&q.info)));
        }
        if !q.location.is_empty() {
            attrs.push(Attribute::new("printer-location", Value::text(&q.location)));
        }
        let req = Request::new(Operation::CUPS_ADD_MODIFY_PRINTER)
            .printer_uri(&Self::queue_uri(&q.name))
            .group(GroupTag::Printer, attrs);
        let limits = Limits::QUERY.with_total(ADD_QUEUE_TIMEOUT);
        self.write("/admin/", req, limits, cancel).await?;
        Ok(())
    }

    /// CUPS-Delete-Printer (CUPS IPP spec, operation 0x4004).
    pub async fn delete_queue(&self, name: &str, cancel: &CancelToken) -> Result<(), CupsError> {
        let req = Request::new(Operation::CUPS_DELETE_PRINTER).printer_uri(&Self::queue_uri(name));
        self.write("/admin/", req, Limits::QUERY, cancel).await?;
        Ok(())
    }

    /// Prints the CUPS test page and returns the job id.
    pub async fn print_test_page(
        &self,
        name: &str,
        cancel: &CancelToken,
    ) -> Result<i32, CupsError> {
        let uri = Self::queue_uri(name);
        let path = format!("/printers/{}", encode_name(name));
        let caps = self
            .read(
                &path,
                Request::new(Operation::GET_PRINTER_ATTRIBUTES)
                    .printer_uri(&uri)
                    .requested_attributes(&["document-format-supported"]),
                cancel,
            )
            .await?;
        let supported = caps
            .first_group(GroupTag::Printer)
            .map(|g| {
                g.strs_of("document-format-supported")
                    .contains(&TEST_PAGE_FORMAT)
            })
            .unwrap_or(false);
        if !supported {
            return Err(CupsError::TestPageUnavailable);
        }
        let req = Request::new(Operation::PRINT_JOB)
            .printer_uri(&uri)
            .op_attr(Attribute::new("job-name", Value::name("Test page")))
            .op_attr(Attribute::new(
                "document-format",
                Value::mime(TEST_PAGE_FORMAT),
            ))
            .document(TEST_PAGE.as_bytes().to_vec());
        let resp = self.write(&path, req, Limits::QUERY, cancel).await?;
        resp.first_group(GroupTag::Job)
            .and_then(|g| g.int_of("job-id"))
            .ok_or(CupsError::Other(IppError::Status {
                status: resp.status,
                message: Some("CUPS did not return a job id".into()),
            }))
    }
}
