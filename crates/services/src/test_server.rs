// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! In-process HTTP/1.1 server for tests. Serves canned responses per
//! "METHOD /path" and records every request it receives.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, UnixListener};

#[derive(Clone, Debug)]
pub struct Canned {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub delay: Duration,
}

impl Canned {
    pub fn ok(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body,
            delay: Duration::ZERO,
        }
    }
    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            delay: Duration::ZERO,
        }
    }
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
    pub fn delayed(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }
}

#[derive(Clone, Debug)]
pub struct Recorded {
    /// Request line and headers exactly as received.
    pub head: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Default)]
pub struct Routes {
    /// Responses per route are served in order; the last one repeats.
    map: Arc<Mutex<HashMap<String, Vec<Canned>>>>,
    pub log: Arc<Mutex<Vec<Recorded>>>,
}

impl Routes {
    pub fn on(&self, route: &str, c: Canned) -> &Self {
        self.map
            .lock()
            .unwrap()
            .entry(route.into())
            .or_default()
            .push(c);
        self
    }

    fn take(&self, route: &str) -> Canned {
        let mut m = self.map.lock().unwrap();
        match m.get_mut(route) {
            Some(v) if v.len() > 1 => v.remove(0),
            Some(v) => v[0].clone(),
            None => Canned::status(404),
        }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }
}

pub async fn serve_tcp(routes: Routes) -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((s, _)) = l.accept().await else { return };
            tokio::spawn(handle(s, routes.clone()));
        }
    });
    port
}

pub async fn serve_unix(routes: Routes, path: &Path) {
    let _ = std::fs::remove_file(path);
    let l = UnixListener::bind(path).unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((s, _)) = l.accept().await else { return };
            tokio::spawn(handle(s, routes.clone()));
        }
    });
}

async fn handle<S: AsyncRead + AsyncWrite + Unpin>(mut s: S, routes: Routes) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        let Ok(n) = s.read(&mut tmp).await else {
            return;
        };
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split(' ');
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let Ok(n) = s.read(&mut tmp).await else {
            return;
        };
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    let route = format!("{method} {path}");
    routes.log.lock().unwrap().push(Recorded {
        head: head.clone(),
        path,
        headers,
        body,
    });
    let c = routes.take(&route);
    tokio::time::sleep(c.delay).await;
    let mut out = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        c.status,
        c.body.len()
    );
    for (k, v) in &c.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let _ = s.write_all(out.as_bytes()).await;
    let _ = s.write_all(&c.body).await;
    let _ = s.shutdown().await;
}

pub fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../protocols/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap()
}
