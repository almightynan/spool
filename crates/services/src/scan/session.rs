// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! eSCL scan jobs (Mopria eSCL Technical Specification sections 7 to 11, as
//! cited by OpenPrinting go-mfp proto/escl/client.go):
//!
//! 1. POST {base}ScanJobs with ScanSettings: 201 Created with Location.
//!    503 means the scanner is busy; 409 that the request conflicts with its
//!    state (for example an empty feeder).
//! 2. GET {job}/NextDocument repeatedly: 200 with one page, 404 when no pages
//!    remain. 503 means the page is not ready yet (HTTP 503 is temporary,
//!    RFC 9110 section 15.6.4), so the request is repeated after a pause.
//! 3. DELETE {job} cancels a running job.

use crate::net::{self, CancelToken, HttpRequest, Limits, NetError};
use hyper::header::CONTENT_TYPE;
use hyper::Method;
use spool_protocols::escl::{
    ScanSettings, ScannerCapabilities, ScannerStatus, SettingsError, XmlError,
};
use spool_protocols::uri::Uri;
use std::fmt;
use std::time::Duration;
use tokio::sync::watch;

/// Pages at 600 dpi color can be tens of megabytes; this bounds memory.
const MAX_PAGE_BYTES: usize = 256 * 1024 * 1024;
/// Scanning one page at high resolution takes a while on consumer devices.
const PAGE_TIMEOUT: Duration = Duration::from_secs(300);
const BUSY_PAUSE: Duration = Duration::from_secs(1);
/// How long NextDocument may keep answering 503 before giving up.
const BUSY_LIMIT: Duration = Duration::from_secs(120);
/// Feeders can hold dozens of pages; this stops a device that never says 404.
const MAX_PAGES: u32 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    Net(NetError),
    Busy,
    /// 409: the scanner refused the settings in its current state.
    Conflict,
    Http(u16),
    Xml(XmlError),
    Settings(SettingsError),
    NoJobLocation,
    NoPages,
    Cancelled,
}

impl From<NetError> for ScanError {
    fn from(e: NetError) -> Self {
        match e {
            NetError::Cancelled => Self::Cancelled,
            e => Self::Net(e),
        }
    }
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Net(e) => e.fmt(f),
            Self::Busy => write!(f, "the scanner is busy with another job"),
            Self::Conflict => write!(
                f,
                "the scanner refused to start; check that paper is loaded and the lid or feeder is closed"
            ),
            Self::Http(c) => write!(f, "the scanner answered HTTP {c}"),
            Self::Xml(e) => write!(f, "the scanner sent an invalid document: {e}"),
            Self::Settings(e) => e.fmt(f),
            Self::NoJobLocation => write!(f, "the scanner did not say where the scan job is"),
            Self::NoPages => write!(f, "the scanner returned no pages"),
            Self::Cancelled => write!(f, "scan canceled"),
        }
    }
}

impl std::error::Error for ScanError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanProgress {
    Starting,
    /// Waiting for page `page` (1-based).
    Scanning {
        page: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedPage {
    pub data: Vec<u8>,
    pub content_type: Option<String>,
    pub dpi: u32,
}

#[derive(Debug, Clone)]
pub struct ScanClient {
    /// Ends with "/", for example "http://192.168.1.147:80/eSCL/".
    base_url: String,
}

impl ScanClient {
    pub fn new(base_url: &str) -> Self {
        let mut b = base_url.to_string();
        if !b.ends_with('/') {
            b.push('/');
        }
        Self { base_url: b }
    }

    async fn get_text(&self, name: &str, cancel: &CancelToken) -> Result<String, ScanError> {
        let req = HttpRequest::to_url(Method::GET, &format!("{}{name}", self.base_url))?;
        let r = net::send(req, Limits::QUERY, cancel).await?;
        if r.status != 200 {
            return Err(ScanError::Http(r.status));
        }
        Ok(String::from_utf8_lossy(&r.body).into_owned())
    }

    pub async fn capabilities(
        &self,
        cancel: &CancelToken,
    ) -> Result<ScannerCapabilities, ScanError> {
        ScannerCapabilities::parse(&self.get_text("ScannerCapabilities", cancel).await?)
            .map_err(ScanError::Xml)
    }

    pub async fn status(&self, cancel: &CancelToken) -> Result<ScannerStatus, ScanError> {
        ScannerStatus::parse(&self.get_text("ScannerStatus", cancel).await?).map_err(ScanError::Xml)
    }

    /// Job URL from the Location header. Only the path is kept and joined with
    /// the base URL's authority: some devices put unresolvable host names or
    /// malformed IPv6 literals in Location (go-mfp client.go, sane-airscan
    /// commit 6e5222c).
    fn job_url(&self, location: &str) -> Option<String> {
        let base = Uri::parse(&self.base_url)?;
        let path = if location.starts_with('/') {
            location.to_string()
        } else {
            Uri::parse(location)?.request_path()
        };
        if path.len() <= 1 {
            return None;
        }
        Some(format!(
            "{}://{}:{}{}",
            base.scheme,
            base.host_for_uri(),
            base.port_or_default(),
            path.trim_end_matches('/')
        ))
    }

    /// Runs one scan job and returns its pages. On cancel, the job is deleted
    /// on the scanner before returning [`ScanError::Cancelled`].
    pub async fn scan(
        &self,
        settings: &ScanSettings,
        progress: &watch::Sender<ScanProgress>,
        cancel: &CancelToken,
    ) -> Result<Vec<ScannedPage>, ScanError> {
        let _ = progress.send(ScanProgress::Starting);
        let req = HttpRequest::to_url(Method::POST, &format!("{}ScanJobs", self.base_url))?
            .header(CONTENT_TYPE, "text/xml")
            .body(settings.to_xml().into_bytes());
        let r = net::send(req, Limits::QUERY, cancel).await?;
        match r.status {
            201 => {}
            503 => return Err(ScanError::Busy),
            409 => return Err(ScanError::Conflict),
            s => return Err(ScanError::Http(s)),
        }
        let job = r
            .location
            .as_deref()
            .and_then(|l| self.job_url(l))
            .ok_or(ScanError::NoJobLocation)?;

        let result = self
            .collect_pages(&job, settings.resolution, progress, cancel)
            .await;
        if matches!(result, Err(ScanError::Cancelled)) {
            // Best effort: the scanner may already have finished the job.
            let _ = self.delete_job(&job).await;
        }
        result
    }

    async fn collect_pages(
        &self,
        job: &str,
        dpi: u32,
        progress: &watch::Sender<ScanProgress>,
        cancel: &CancelToken,
    ) -> Result<Vec<ScannedPage>, ScanError> {
        let mut pages = Vec::new();
        let limits = Limits::QUERY
            .with_total(PAGE_TIMEOUT)
            .with_max_body(MAX_PAGE_BYTES);
        let mut busy_since: Option<tokio::time::Instant> = None;
        while pages.len() < MAX_PAGES as usize {
            let _ = progress.send(ScanProgress::Scanning {
                page: pages.len() as u32 + 1,
            });
            let req = HttpRequest::to_url(Method::GET, &format!("{job}/NextDocument"))?;
            let r = net::send(req, limits, cancel).await?;
            match r.status {
                200 => {
                    busy_since = None;
                    pages.push(ScannedPage {
                        data: r.body,
                        content_type: r.content_type,
                        dpi,
                    });
                }
                404 => break,
                503 => {
                    let since = *busy_since.get_or_insert_with(tokio::time::Instant::now);
                    if since.elapsed() > BUSY_LIMIT {
                        return Err(ScanError::Busy);
                    }
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(ScanError::Cancelled),
                        _ = tokio::time::sleep(BUSY_PAUSE) => {}
                    }
                }
                409 if pages.is_empty() => return Err(ScanError::Conflict),
                s => return Err(ScanError::Http(s)),
            }
        }
        if pages.is_empty() {
            Err(ScanError::NoPages)
        } else {
            Ok(pages)
        }
    }

    async fn delete_job(&self, job: &str) -> Result<(), ScanError> {
        let req = HttpRequest::to_url(Method::DELETE, job)?;
        net::send(req, Limits::QUERY, &CancelToken::new()).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::jpeg::test_jpeg;
    use crate::test_server::{serve_tcp, Canned, Routes};
    use spool_protocols::escl::{ColorMode, InputSource, PaperSize, ScanRequest};

    fn caps_fixture() -> ScannerCapabilities {
        let x = crate::test_server::fixture("escl-ScannerCapabilities.xml");
        ScannerCapabilities::parse(&String::from_utf8(x).unwrap()).unwrap()
    }

    fn settings() -> ScanSettings {
        ScanSettings::for_request(
            &caps_fixture(),
            &ScanRequest {
                source: InputSource::Platen,
                duplex: false,
                color_mode: ColorMode::Rgb24,
                resolution: 75,
                paper: PaperSize::A4,
                document_format: "image/jpeg".into(),
                intent: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn job_url_keeps_only_the_path() {
        let c = ScanClient::new("http://192.168.1.147:80/eSCL");
        assert_eq!(
            c.job_url("http://HPXXXX.local:8080/eSCL/ScanJobs/abc/")
                .as_deref(),
            Some("http://192.168.1.147:80/eSCL/ScanJobs/abc")
        );
        assert_eq!(
            c.job_url("/eSCL/ScanJobs/7").as_deref(),
            Some("http://192.168.1.147:80/eSCL/ScanJobs/7")
        );
        assert_eq!(c.job_url("/"), None);
    }

    #[tokio::test]
    async fn pages_until_404_with_busy_pause() {
        let routes = Routes::default();
        routes.on(
            "POST /eSCL/ScanJobs",
            Canned::status(201).header("Location", "/eSCL/ScanJobs/1"),
        );
        routes.on("GET /eSCL/ScanJobs/1/NextDocument", Canned::status(503));
        routes.on(
            "GET /eSCL/ScanJobs/1/NextDocument",
            Canned::ok(test_jpeg(620, 877, 3)).header("Content-Type", "image/jpeg"),
        );
        routes.on("GET /eSCL/ScanJobs/1/NextDocument", Canned::status(404));
        let port = serve_tcp(routes.clone()).await;
        let c = ScanClient::new(&format!("http://127.0.0.1:{port}/eSCL/"));
        let (tx, _rx) = watch::channel(ScanProgress::Starting);
        let pages = c.scan(&settings(), &tx, &CancelToken::new()).await.unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].content_type.as_deref(), Some("image/jpeg"));
        assert_eq!(pages[0].dpi, 75);
        let posted = &routes.requests()[0];
        let body = String::from_utf8_lossy(&posted.body);
        assert!(body.contains("<scan:XResolution>75</scan:XResolution>"));
        assert!(posted.head.contains("Content-Type: text/xml"));
    }

    #[tokio::test]
    async fn busy_and_conflict_are_typed() {
        let routes = Routes::default();
        routes.on("POST /eSCL/ScanJobs", Canned::status(503));
        routes.on("POST /eSCL/ScanJobs", Canned::status(409));
        let port = serve_tcp(routes).await;
        let c = ScanClient::new(&format!("http://127.0.0.1:{port}/eSCL/"));
        let (tx, _rx) = watch::channel(ScanProgress::Starting);
        assert_eq!(
            c.scan(&settings(), &tx, &CancelToken::new()).await,
            Err(ScanError::Busy)
        );
        assert_eq!(
            c.scan(&settings(), &tx, &CancelToken::new()).await,
            Err(ScanError::Conflict)
        );
    }

    #[tokio::test]
    async fn cancel_deletes_the_job() {
        let routes = Routes::default();
        routes.on(
            "POST /eSCL/ScanJobs",
            Canned::status(201).header("Location", "/eSCL/ScanJobs/9"),
        );
        routes.on(
            "GET /eSCL/ScanJobs/9/NextDocument",
            Canned::ok(vec![]).delayed(Duration::from_secs(30)),
        );
        routes.on("DELETE /eSCL/ScanJobs/9", Canned::status(200));
        let port = serve_tcp(routes.clone()).await;
        let c = ScanClient::new(&format!("http://127.0.0.1:{port}/eSCL/"));
        let (tx, _rx) = watch::channel(ScanProgress::Starting);
        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            c2.cancel();
        });
        assert_eq!(
            c.scan(&settings(), &tx, &cancel).await,
            Err(ScanError::Cancelled)
        );
        assert!(routes
            .requests()
            .iter()
            .any(|r| r.head.starts_with("DELETE /eSCL/ScanJobs/9 ")));
    }

    #[tokio::test]
    async fn capabilities_and_status_against_fixtures() {
        let routes = Routes::default();
        routes.on(
            "GET /eSCL/ScannerCapabilities",
            Canned::ok(crate::test_server::fixture("escl-ScannerCapabilities.xml")),
        );
        routes.on(
            "GET /eSCL/ScannerStatus",
            Canned::ok(crate::test_server::fixture("escl-ScannerStatus.xml")),
        );
        let port = serve_tcp(routes).await;
        let c = ScanClient::new(&format!("http://127.0.0.1:{port}/eSCL/"));
        let t = CancelToken::new();
        assert_eq!(c.capabilities(&t).await.unwrap().version, "2.1");
        assert!(c.status(&t).await.unwrap().state.can_scan());
    }
}
