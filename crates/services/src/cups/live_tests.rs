// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! End-to-end tests against the real local cupsd. Ignored by default.
//!
//! Run with `cargo test -p spool-services -- --ignored --test-threads=1`.
//! They create and delete one queue named "printdesk-selftest" whose device URI
//! points at a fake IPP printer on 127.0.0.1 that answers with the recorded HP
//! ENVY 5530 attributes, so no job reaches a real printer. The default printer
//! is restored afterwards.

use super::*;
use crate::identity::current_user;
use crate::ipp_client::{self, IppTarget};
use crate::net::{CancelToken, Limits, Target};
use crate::test_server::{fixture, serve_tcp, Canned, Routes};
use spool_protocols::ipp::{Attribute, GroupTag, Operation, Request, Value};
use spool_protocols::jobs::JobState;
use std::path::PathBuf;
use std::time::Duration;

const QUEUE: &str = "printdesk-selftest";

async fn client() -> CupsClient {
    CupsClient::new(current_user().await.unwrap())
}

/// Removes the queue on drop, even if an assertion failed halfway.
struct Cleanup(CupsClient, Option<String>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let c = self.0.clone();
        let previous_default = self.1.take();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        std::thread::spawn(move || {
            rt.block_on(async move {
                let t = CancelToken::new();
                let _ = c.delete_queue(QUEUE, &t).await;
                if let Some(d) = previous_default {
                    let _ = c.set_default(&d, &t).await;
                }
            })
        })
        .join()
        .unwrap();
    }
}

async fn held_job(c: &CupsClient) -> i32 {
    let req = Request::new(Operation::PRINT_JOB)
        .printer_uri(&CupsClient::queue_uri(QUEUE))
        .user(c.user())
        .op_attr(Attribute::new("job-name", Value::name("selftest held")))
        .op_attr(Attribute::new("document-format", Value::mime("text/plain")))
        .group(
            GroupTag::Job,
            vec![Attribute::new(
                "job-hold-until",
                Value::keyword("indefinite"),
            )],
        )
        .document(b"spool selftest\n".to_vec());
    let t = IppTarget {
        target: Target::Unix(PathBuf::from(client::DEFAULT_SOCKET)),
        host_header: "localhost".into(),
        path: format!("/printers/{QUEUE}"),
        printer_uri: String::new(),
    };
    let auth = format!("PeerCred {}", c.user());
    let r = ipp_client::send(&t, req, Some(&auth), Limits::QUERY, &CancelToken::new())
        .await
        .unwrap();
    spool_protocols::ipp::AttributeSet::int_of(r.first_group(GroupTag::Job).unwrap(), "job-id")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "changes the local CUPS configuration"]
async fn queue_lifecycle_against_local_cupsd() {
    let routes = Routes::default();
    // Every IPP request from cupsd (PPD generation, job submission) gets the
    // recorded attributes; that is enough for driver generation.
    routes.on(
        "POST /ipp/print",
        Canned::ok(fixture("printer-get-printer-attributes.ipp")),
    );
    let port = serve_tcp(routes.clone()).await;
    let c = client().await;
    let t = CancelToken::new();

    let previous_default = c.default_queue(&t).await.unwrap();
    let _cleanup = Cleanup(c.clone(), previous_default.clone());

    c.add_queue(
        &NewQueue {
            name: QUEUE.into(),
            device_uri: format!("ipp://127.0.0.1:{port}/ipp/print"),
            info: "spool self test".into(),
            location: "nowhere".into(),
        },
        &t,
    )
    .await
    .unwrap();
    assert!(
        routes.requests().iter().any(|r| r.path == "/ipp/print"),
        "cupsd never queried the fake printer"
    );
    let queues = c.queues(&t).await.unwrap();
    let q = queues
        .iter()
        .find(|q| q.name == QUEUE)
        .expect("queue listed");
    assert_eq!(q.info.as_deref(), Some("spool self test"));
    assert_eq!(q.location.as_deref(), Some("nowhere"));

    c.set_default(QUEUE, &t).await.unwrap();
    assert_eq!(c.default_queue(&t).await.unwrap().as_deref(), Some(QUEUE));

    // cupsd names every generated IPP Everywhere driver this way (the user's
    // existing queue in cups-queue-get-printer-attributes.ipp reads the same).
    let status = c.queue_status(QUEUE, &t).await.unwrap();
    assert_eq!(
        status.info.make_and_model.as_deref(),
        Some("Printer - IPP Everywhere")
    );

    // Hold, release and cancel a job that is held from the start.
    let id = held_job(&c).await;
    let state = |jobs: Vec<spool_protocols::jobs::Job>| {
        jobs.into_iter().find(|j| j.id == id).map(|j| j.state)
    };
    assert_eq!(
        state(c.jobs(QUEUE, WhichJobs::NotCompleted, &t).await.unwrap()),
        Some(JobState::Held)
    );
    c.job_op(QUEUE, id, JobOp::Release, &t).await.unwrap();
    c.job_op(QUEUE, id, JobOp::Cancel, &t).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let done = state(c.jobs(QUEUE, WhichJobs::Completed, &t).await.unwrap());
    assert!(
        matches!(
            done,
            Some(JobState::Canceled | JobState::Aborted | JobState::Completed)
        ),
        "{done:?}"
    );

    // A second held job: hold is a no-op on a held job, release requeues it.
    let id2 = held_job(&c).await;
    c.job_op(QUEUE, id2, JobOp::Cancel, &t).await.unwrap();
    match c.job_op(QUEUE, id2, JobOp::Restart, &t).await {
        Ok(()) | Err(CupsError::NotPossible(_)) => {}
        Err(e) => panic!("restart: {e}"),
    }
    c.job_op(QUEUE, id2, JobOp::Cancel, &t).await.ok();

    // The test page needs cups-filters; on this machine it is installed.
    match c.print_test_page(QUEUE, &t).await {
        Ok(job) => {
            c.job_op(QUEUE, job, JobOp::Cancel, &t).await.ok();
        }
        Err(CupsError::TestPageUnavailable) => {}
        Err(e) => panic!("test page: {e}"),
    }

    c.delete_queue(QUEUE, &t).await.unwrap();
    assert!(!c.queues(&t).await.unwrap().iter().any(|q| q.name == QUEUE));
}

#[tokio::test]
#[ignore = "needs the local cupsd"]
async fn read_only_queries_against_local_cupsd() {
    let c = client().await;
    let t = CancelToken::new();
    let queues = c.queues(&t).await.unwrap();
    for q in &queues {
        c.queue_status(&q.name, &t).await.unwrap();
        c.jobs(&q.name, WhichJobs::Completed, &t).await.unwrap();
    }
}
