// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! IPP over HTTP (RFC 8010 section 4: POST with Content-Type application/ipp).

use crate::net::{self, CancelToken, HttpRequest, Limits, NetError, Target};
use hyper::header::{HeaderName, AUTHORIZATION, CONTENT_TYPE};
use hyper::Method;
use spool_protocols::ipp::{DecodeError, Request, Response, StatusCode, FALLBACK_VERSION};
use std::fmt;

/// Where an IPP request goes. `printer_uri` is the value of the printer-uri
/// operation attribute; it can differ from the transport (CUPS over a socket).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IppTarget {
    pub target: Target,
    pub host_header: String,
    pub path: String,
    pub printer_uri: String,
}

impl IppTarget {
    /// Target for an ipp:// or ipps:// printer URI, sent directly to the device.
    pub fn for_uri(uri: &str) -> Result<Self, NetError> {
        let http = HttpRequest::to_url(Method::POST, uri)?;
        Ok(Self {
            target: http.target,
            host_header: http.host_header,
            path: http.path,
            printer_uri: uri.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IppError {
    Net(NetError),
    /// The HTTP layer refused the request (for example 401, 403, 404).
    Http(u16),
    Decode(DecodeError),
    /// The IPP response had an error status.
    Status {
        status: StatusCode,
        message: Option<String>,
    },
}

impl IppError {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Self::Status { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn is_unreachable(&self) -> bool {
        matches!(self, Self::Net(e) if e.is_unreachable())
    }
}

impl From<NetError> for IppError {
    fn from(e: NetError) -> Self {
        Self::Net(e)
    }
}

impl fmt::Display for IppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Net(e) => e.fmt(f),
            Self::Http(code) => write!(f, "the server answered HTTP {code}"),
            Self::Decode(e) => write!(f, "the response is not valid IPP: {e}"),
            Self::Status { status, message } => match message {
                Some(m) if !m.is_empty() => write!(f, "{m} ({status})"),
                _ => write!(f, "{status}"),
            },
        }
    }
}

impl std::error::Error for IppError {}

/// Sends one IPP request. Error statuses become [`IppError::Status`].
///
/// If the server answers server-error-version-not-supported to an IPP/2.0
/// request, the request is sent once more as IPP/1.1 (RFC 8011 section 4.1.8).
pub async fn send(
    target: &IppTarget,
    req: Request,
    auth: Option<&str>,
    limits: Limits,
    cancel: &CancelToken,
) -> Result<Response, IppError> {
    let resp = send_once(target, &req, auth, limits, cancel).await?;
    if resp.status == StatusCode::VERSION_NOT_SUPPORTED && req.version != FALLBACK_VERSION {
        let retry = req.with_version(FALLBACK_VERSION);
        return check(send_once(target, &retry, auth, limits, cancel).await?);
    }
    check(resp)
}

fn check(resp: Response) -> Result<Response, IppError> {
    if resp.status.is_success() {
        Ok(resp)
    } else {
        Err(IppError::Status {
            status: resp.status,
            message: resp.status_message().map(str::to_string),
        })
    }
}

async fn send_once(
    target: &IppTarget,
    req: &Request,
    auth: Option<&str>,
    limits: Limits,
    cancel: &CancelToken,
) -> Result<Response, IppError> {
    let mut headers: Vec<(HeaderName, String)> = vec![(CONTENT_TYPE, "application/ipp".into())];
    if let Some(a) = auth {
        headers.push((AUTHORIZATION, a.to_string()));
    }
    let http = HttpRequest {
        method: Method::POST,
        target: target.target.clone(),
        host_header: target.host_header.clone(),
        path: target.path.clone(),
        headers,
        body: req.encode(),
    };
    let r = net::send(http, limits, cancel).await?;
    if r.status != 200 {
        return Err(IppError::Http(r.status));
    }
    Response::decode(&r.body).map_err(IppError::Decode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{fixture, serve_tcp, Canned, Routes};
    use spool_protocols::ipp::{GroupTag, Message, Operation};

    fn status_only(code: u16) -> Vec<u8> {
        Message {
            version: (2, 0),
            code,
            request_id: 1,
            groups: vec![],
            data: vec![],
        }
        .encode()
    }

    #[tokio::test]
    async fn get_printer_attributes_against_fixture() {
        let routes = Routes::default();
        routes.on(
            "POST /ipp/print",
            Canned::ok(fixture("printer-get-printer-attributes.ipp")),
        );
        let port = serve_tcp(routes.clone()).await;
        let t = IppTarget::for_uri(&format!("ipp://127.0.0.1:{port}/ipp/print")).unwrap();
        let req = Request::new(Operation::GET_PRINTER_ATTRIBUTES).printer_uri(&t.printer_uri);
        let resp = send(&t, req, None, Limits::QUERY, &CancelToken::new())
            .await
            .unwrap();
        assert!(resp.first_group(GroupTag::Printer).is_some());
        let sent = &routes.requests()[0];
        assert!(sent
            .headers
            .iter()
            .any(|(k, v)| k == "content-type" && v == "application/ipp"));
        let decoded = Message::decode(&sent.body).unwrap();
        assert_eq!(decoded.code, Operation::GET_PRINTER_ATTRIBUTES.0);
    }

    #[tokio::test]
    async fn version_fallback_once() {
        let routes = Routes::default();
        routes.on("POST /ipp/print", Canned::ok(status_only(0x0503)));
        routes.on(
            "POST /ipp/print",
            Canned::ok(fixture("printer-get-printer-attributes.ipp")),
        );
        let port = serve_tcp(routes.clone()).await;
        let t = IppTarget::for_uri(&format!("ipp://127.0.0.1:{port}/ipp/print")).unwrap();
        let req = Request::new(Operation::GET_PRINTER_ATTRIBUTES).printer_uri(&t.printer_uri);
        send(&t, req, None, Limits::QUERY, &CancelToken::new())
            .await
            .unwrap();
        let reqs = routes.requests();
        assert_eq!(reqs.len(), 2);
        assert_eq!(&reqs[0].body[..2], &[2, 0]);
        assert_eq!(&reqs[1].body[..2], &[1, 1]);
    }

    #[tokio::test]
    async fn error_status_is_typed() {
        let routes = Routes::default();
        routes.on(
            "POST /printers/x",
            Canned::ok(fixture("cups-not-found.ipp")),
        );
        routes.on("POST /admin/", Canned::status(401));
        let port = serve_tcp(routes).await;
        let t = IppTarget::for_uri(&format!("ipp://127.0.0.1:{port}/printers/x")).unwrap();
        let e = send(
            &t,
            Request::new(Operation::GET_PRINTER_ATTRIBUTES),
            None,
            Limits::QUERY,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(e.status(), Some(StatusCode::NOT_FOUND));
        let t = IppTarget::for_uri(&format!("ipp://127.0.0.1:{port}/admin/")).unwrap();
        let e = send(
            &t,
            Request::new(Operation::CUPS_ADD_MODIFY_PRINTER),
            None,
            Limits::QUERY,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(e, IppError::Http(401));
    }
}
