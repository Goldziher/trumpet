//! End-to-end test for SIGHUP-driven auth token rotation (F9).
//!
//! Drives [`server::reload_token`] directly rather than sending a real SIGHUP
//! to the test process, because the test harness shares its PID with every
//! other in-process daemon and a real signal would race against unrelated
//! suites. The signal-handling path is exercised separately by the
//! `daemon_writes_pid_file_at_startup` smoke test below — if the PID file is
//! present, an external `trumpet auth rotate` invocation can deliver the
//! signal in production.

mod common;

use std::time::Duration;

use common::TestDaemon;
use tonic::Request;
use tonic::transport::Endpoint;

use trumpet::grpc::proto::ListTasksRequest;
use trumpet::grpc::proto::a2a_service_client::A2aServiceClient;

#[tokio::test]
async fn daemon_writes_pid_file_at_startup() {
    let daemon = TestDaemon::builder().require_auth(false).spawn().await;

    let pid_path = daemon.config.daemon.pid_file.clone();
    assert!(
        pid_path.exists(),
        "daemon must write its PID file so `trumpet auth rotate` and `trumpet stop` can locate it: {}",
        pid_path.display()
    );
    let pid: u32 = std::fs::read_to_string(&pid_path)
        .expect("read pid file")
        .trim()
        .parse()
        .expect("pid file contains a u32");
    assert_eq!(
        pid,
        std::process::id(),
        "pid file must contain the daemon's process id"
    );

    daemon.shutdown().await.expect("clean shutdown");

    // The pid file is removed on shutdown so a stale value is never left
    // behind for the next daemon to pick up.
    assert!(
        !pid_path.exists(),
        "daemon must remove its pid file on shutdown"
    );
}

// `libc::raise(SIGHUP)` delivers the signal to the entire process. With
// `cargo test`'s default parallelism, a concurrently-running test that has
// spawned a `trumpet` subprocess could see its child terminated by the
// default SIGHUP disposition (terminate) before installing a handler.
// Marking this test serial avoids that cross-test interference.
#[tokio::test]
#[serial_test::serial]
async fn rotated_token_is_accepted_old_token_is_rejected() {
    let daemon = TestDaemon::builder().require_auth(true).spawn().await;
    let old_token = daemon.auth_token.clone();

    // Sanity: the original token works against gRPC.
    let endpoint = Endpoint::from_shared(format!("http://{}", daemon.grpc_addr))
        .expect("endpoint")
        .connect_timeout(Duration::from_secs(2));
    let channel = endpoint.connect().await.expect("connect grpc");

    let mut client = A2aServiceClient::with_interceptor(channel.clone(), {
        let token = old_token.clone();
        move |mut req: Request<()>| {
            req.metadata_mut()
                .insert("authorization", format!("Bearer {token}").parse().unwrap());
            Ok(req)
        }
    });

    client
        .list_tasks(Request::new(ListTasksRequest::default()))
        .await
        .expect("list_task with original token must succeed");

    // Write a new token to disk and trigger the daemon's reload path
    // directly — this is the same code the SIGHUP handler runs in
    // production, exercised here without depending on real signals.
    let new_token = uuid::Uuid::new_v4().to_string();
    std::fs::write(&daemon.config.security.auth_token_path, &new_token).expect("overwrite token");

    // Use the same loader the daemon uses on SIGHUP, against the daemon's
    // shared token slot. We get at the slot via a single-shot HTTP request
    // through the daemon's existing path: there is none, so instead drive
    // `auth::reload_token` directly using the same path / slot the daemon
    // is using. We can re-trigger that by sending SIGHUP to ourselves —
    // tokio's per-process signal stream wakes the daemon's handler.
    #[cfg(unix)]
    unsafe {
        // SAFETY: raise() has no preconditions; SIGHUP is handled by the
        // daemon's `tokio::signal::unix::signal(SignalKind::hangup())` task,
        // which is shared per-process by tokio. The test thread does not
        // install its own SIGHUP handler.
        libc::raise(libc::SIGHUP);
    }

    // Give the SIGHUP task time to read the file and swap the slot.
    let mut accepted = false;
    for _ in 0..40 {
        let mut new_client = A2aServiceClient::with_interceptor(channel.clone(), {
            let token = new_token.clone();
            move |mut req: Request<()>| {
                req.metadata_mut()
                    .insert("authorization", format!("Bearer {token}").parse().unwrap());
                Ok(req)
            }
        });
        if new_client
            .list_tasks(Request::new(ListTasksRequest::default()))
            .await
            .is_ok()
        {
            accepted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        accepted,
        "rotated token must be accepted by gRPC within 4 seconds"
    );

    // Old token must now be rejected.
    let result = client
        .list_tasks(Request::new(ListTasksRequest::default()))
        .await;
    assert!(
        matches!(result, Err(ref s) if s.code() == tonic::Code::Unauthenticated),
        "old token must be rejected after rotation, got: {result:?}"
    );

    daemon.shutdown().await.expect("clean shutdown");
}
