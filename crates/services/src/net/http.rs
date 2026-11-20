// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! HTTP/1.1 client over TCP, TLS or a Unix socket (hyper connection API).
//!
//! One connection per request: polling rates are low and printers handle
//! keep-alive inconsistently. Every request has a connect timeout, an overall
//! timeout, a body size limit and a cancel token.

use super::cancel::CancelToken;
use super::error::{ConnectFailure, NetError};
use super::tls::device_client_config;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Bytes;
use hyper::header::{HeaderName, HeaderValue, CONTENT_LENGTH, HOST};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use spool_protocols::uri::Uri;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpStream, UnixStream};
use tokio_rustls::TlsConnector;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Tcp { host: String, port: u16, tls: bool },
    Unix(PathBuf),
}

impl Target {
    fn describe(&self) -> String {
        match self {
            Self::Tcp { host, port, .. } => format!("{host}:{port}"),
            Self::Unix(p) => p.display().to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub target: Target,
    pub host_header: String,
    /// Path and query, starting with "/".
    pub path: String,
    pub headers: Vec<(HeaderName, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// Request to an http, https, ipp or ipps URL.
    pub fn to_url(method: Method, url: &str) -> Result<Self, NetError> {
        let u =
            Uri::parse(url).ok_or_else(|| NetError::InvalidRequest(format!("bad URL {url}")))?;
        if !matches!(u.scheme.as_str(), "http" | "https" | "ipp" | "ipps") {
            return Err(NetError::InvalidRequest(format!("unsupported URL {url}")));
        }
        if u.host.is_empty() {
            return Err(NetError::InvalidRequest(format!("URL has no host: {url}")));
        }
        let port = u.port_or_default();
        Ok(Self {
            method,
            host_header: format!("{}:{port}", u.host_for_uri()),
            target: Target::Tcp {
                host: u.host.clone(),
                port,
                tls: u.is_tls(),
            },
            path: u.request_path(),
            headers: Vec::new(),
            body: Vec::new(),
        })
    }

    pub fn header(mut self, name: HeaderName, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub connect: Duration,
    pub total: Duration,
    pub max_body: usize,
}

impl Limits {
    /// Status queries and small documents.
    pub const QUERY: Limits = Limits {
        connect: Duration::from_secs(5),
        total: Duration::from_secs(10),
        max_body: 4 * 1024 * 1024,
    };

    pub fn with_total(mut self, total: Duration) -> Self {
        self.total = total;
        self
    }

    pub fn with_max_body(mut self, max: usize) -> Self {
        self.max_body = max;
        self
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub location: Option<String>,
    pub content_type: Option<String>,
    pub www_authenticate: Option<String>,
    pub body: Vec<u8>,
}

/// Sends a request and reads the whole response body.
pub async fn send(
    req: HttpRequest,
    limits: Limits,
    cancel: &CancelToken,
) -> Result<HttpResponse, NetError> {
    if cancel.is_cancelled() {
        return Err(NetError::Cancelled);
    }
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(NetError::Cancelled),
        r = tokio::time::timeout(limits.total, exchange(req, limits)) => {
            r.unwrap_or(Err(NetError::Timeout(limits.total)))
        }
    }
}

async fn exchange(req: HttpRequest, limits: Limits) -> Result<HttpResponse, NetError> {
    let what = req.target.describe();
    match &req.target {
        Target::Unix(path) => {
            let s = tokio::time::timeout(limits.connect, UnixStream::connect(path))
                .await
                .map_err(|_| NetError::Timeout(limits.connect))?
                .map_err(|e| NetError::from_io(&what, &e))?;
            over(s, req, limits).await
        }
        Target::Tcp { host, port, tls } => {
            let tcp = tokio::time::timeout(limits.connect, connect_tcp(host, *port))
                .await
                .map_err(|_| NetError::Timeout(limits.connect))??;
            if *tls {
                let cfg = device_client_config().map_err(|e| NetError::Tls(e.to_string()))?;
                let name = server_name(host)?;
                let s = tokio::time::timeout(
                    limits.connect,
                    TlsConnector::from(cfg).connect(name, tcp),
                )
                .await
                .map_err(|_| NetError::Timeout(limits.connect))?
                .map_err(|e| NetError::Tls(e.to_string()))?;
                over(s, req, limits).await
            } else {
                over(tcp, req, limits).await
            }
        }
    }
}

fn server_name(host: &str) -> Result<ServerName<'static>, NetError> {
    let h = host.split('%').next().unwrap_or(host);
    ServerName::try_from(h.to_string()).map_err(|e| NetError::Tls(e.to_string()))
}

async fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, NetError> {
    let target = format!("{host}:{port}");
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| NetError::Connect {
            target: host.to_string(),
            failure: ConnectFailure::UnknownHost,
        })?
        .collect();
    let mut last = NetError::Connect {
        target: host.to_string(),
        failure: ConnectFailure::UnknownHost,
    };
    for a in addrs {
        match TcpStream::connect(a).await {
            Ok(s) => {
                // Small request/response exchanges; avoid Nagle delays.
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => last = NetError::from_io(&target, &e),
        }
    }
    Err(last)
}

/// Aborts the connection driver when the exchange ends or is dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn over<S>(io: S, req: HttpRequest, limits: Limits) -> Result<HttpResponse, NetError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let proto = |e: hyper::Error| NetError::Protocol(e.to_string());
    // Title-case header names: HP's embedded server never answers IPP requests
    // with lowercase names (seen on the HP ENVY 5530), although RFC 9110
    // section 5.1 makes them case-insensitive.
    let (mut sender, conn) = hyper::client::conn::http1::Builder::new()
        .title_case_headers(true)
        .handshake(TokioIo::new(io))
        .await
        .map_err(proto)?;
    let _driver = AbortOnDrop(tokio::spawn(async move {
        let _ = conn.await;
    }));

    let mut builder = Request::builder()
        .method(req.method.clone())
        .uri(req.path.as_str())
        .header(HOST, req.host_header.as_str());
    for (k, v) in &req.headers {
        let v = HeaderValue::from_str(v).map_err(|e| NetError::InvalidRequest(e.to_string()))?;
        builder = builder.header(k, v);
    }
    if !req.body.is_empty() || req.method == Method::POST {
        builder = builder.header(CONTENT_LENGTH, req.body.len());
    }
    let request = builder
        .body(Full::new(Bytes::from(req.body)))
        .map_err(|e| NetError::InvalidRequest(e.to_string()))?;

    let resp = sender.send_request(request).await.map_err(proto)?;
    let status = resp.status().as_u16();
    let header = |n: HeaderName| {
        resp.headers()
            .get(n)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let location = header(hyper::header::LOCATION);
    let content_type = header(hyper::header::CONTENT_TYPE);
    let www_authenticate = header(hyper::header::WWW_AUTHENTICATE);
    let body = Limited::new(resp.into_body(), limits.max_body)
        .collect()
        .await
        .map_err(|e| {
            if e.downcast_ref::<http_body_util::LengthLimitError>()
                .is_some()
            {
                NetError::BodyTooLarge(limits.max_body)
            } else {
                NetError::Protocol(e.to_string())
            }
        })?
        .to_bytes()
        .to_vec();
    Ok(HttpResponse {
        status,
        location,
        content_type,
        www_authenticate,
        body,
    })
}
