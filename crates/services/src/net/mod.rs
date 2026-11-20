// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Network transport: HTTP/1.1 over TCP, TLS or Unix sockets, with timeouts,
//! cancellation and typed errors.

pub mod cancel;
pub mod error;
pub mod http;
pub mod tls;

pub use cancel::CancelToken;
pub use error::{ConnectFailure, NetError};
pub use http::{send, HttpRequest, HttpResponse, Limits, Target};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{serve_tcp, serve_unix, Canned, Routes};
    use hyper::Method;
    use std::time::Duration;

    #[tokio::test]
    async fn get_and_post() {
        let routes = Routes::default();
        routes.on(
            "GET /a",
            Canned::ok(b"hello".to_vec()).header("Location", "/x"),
        );
        let port = serve_tcp(routes.clone()).await;
        let r = send(
            HttpRequest::to_url(Method::GET, &format!("http://127.0.0.1:{port}/a")).unwrap(),
            Limits::QUERY,
            &CancelToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"hello");
        assert_eq!(r.location.as_deref(), Some("/x"));
        let req = &routes.requests()[0];
        assert_eq!(req.path, "/a");
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "host" && v == &format!("127.0.0.1:{port}")));
        // HP embedded servers ignore lowercase header names.
        assert!(
            req.head
                .contains(&format!("\r\nHost: 127.0.0.1:{port}\r\n")),
            "{}",
            req.head
        );
    }

    #[tokio::test]
    async fn timeout_is_typed() {
        let routes = Routes::default();
        routes.on(
            "GET /slow",
            Canned::ok(vec![]).delayed(Duration::from_secs(5)),
        );
        let port = serve_tcp(routes).await;
        let limits = Limits::QUERY.with_total(Duration::from_millis(200));
        let e = send(
            HttpRequest::to_url(Method::GET, &format!("http://127.0.0.1:{port}/slow")).unwrap(),
            limits,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(e, NetError::Timeout(Duration::from_millis(200)));
        assert!(e.is_unreachable());
    }

    #[tokio::test]
    async fn cancel_interrupts() {
        let routes = Routes::default();
        routes.on(
            "GET /slow",
            Canned::ok(vec![]).delayed(Duration::from_secs(5)),
        );
        let port = serve_tcp(routes).await;
        let c = CancelToken::new();
        let c2 = c.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            c2.cancel();
        });
        let e = send(
            HttpRequest::to_url(Method::GET, &format!("http://127.0.0.1:{port}/slow")).unwrap(),
            Limits::QUERY,
            &c,
        )
        .await
        .unwrap_err();
        assert_eq!(e, NetError::Cancelled);
    }

    #[tokio::test]
    async fn refused_and_too_large() {
        // Port 9 on loopback has nothing listening in test environments.
        let e = send(
            HttpRequest::to_url(Method::GET, "http://127.0.0.1:9/").unwrap(),
            Limits::QUERY,
            &CancelToken::new(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                e,
                NetError::Connect {
                    failure: ConnectFailure::Refused,
                    ..
                }
            ),
            "{e:?}"
        );

        let routes = Routes::default();
        routes.on("GET /big", Canned::ok(vec![0; 2048]));
        let port = serve_tcp(routes).await;
        let e = send(
            HttpRequest::to_url(Method::GET, &format!("http://127.0.0.1:{port}/big")).unwrap(),
            Limits::QUERY.with_max_body(1024),
            &CancelToken::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(e, NetError::BodyTooLarge(1024));
    }

    #[tokio::test]
    async fn unix_socket() {
        let dir = std::env::temp_dir().join(format!("spool-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.sock");
        let routes = Routes::default();
        routes.on("POST /admin/", Canned::ok(b"ok".to_vec()));
        serve_unix(routes.clone(), &path).await;
        let req = HttpRequest {
            method: Method::POST,
            target: Target::Unix(path.clone()),
            host_header: "localhost".into(),
            path: "/admin/".into(),
            headers: vec![],
            body: b"x".to_vec(),
        };
        let r = send(req, Limits::QUERY, &CancelToken::new()).await.unwrap();
        assert_eq!(r.body, b"ok");
        assert_eq!(routes.requests()[0].body, b"x");
        let missing = HttpRequest {
            method: Method::GET,
            target: Target::Unix(dir.join("none.sock")),
            host_header: "localhost".into(),
            path: "/".into(),
            headers: vec![],
            body: vec![],
        };
        let e = send(missing, Limits::QUERY, &CancelToken::new())
            .await
            .unwrap_err();
        assert!(matches!(
            e,
            NetError::Connect {
                failure: ConnectFailure::NoSocket,
                ..
            }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
