//! The M2 transport gate (docs/PLAN.md), run against a disposable local
//! VM — never anything in the production inventory:
//!
//!   RUXEL_TEST_SSH_DEST=<ssh destination> \
//!   RUXEL_TEST_AGENT_BIN=<agent binary built for the VM's arch> \
//!   cargo test -p ruxel-cli --test transport_gate -- --ignored --nocapture
//!
//! Asserts: cold connect → handshake → facts → clean shutdown; agent
//! re-upload skipped when the hash is already present; handshake time
//! after the first run stays under the 1 s budget.

use std::time::Instant;

fn env_or_skip() -> Option<(String, std::path::PathBuf)> {
    let dest = std::env::var("RUXEL_TEST_SSH_DEST").ok()?;
    let bin = std::env::var("RUXEL_TEST_AGENT_BIN").ok()?;
    Some((dest, bin.into()))
}

#[tokio::test]
#[ignore = "needs RUXEL_TEST_SSH_DEST + RUXEL_TEST_AGENT_BIN (local VM)"]
async fn cold_connect_handshake_facts_shutdown() {
    let Some((dest, bin)) = env_or_skip() else {
        eprintln!("env not set — skipped");
        return;
    };

    // Run 1: possibly uploads the agent.
    let t0 = Instant::now();
    let (conn, host) = ruxel_cli::transport::connect(&dest, &bin, "gate-run-1", false)
        .await
        .expect("first connect");
    let first_connect = t0.elapsed();
    eprintln!(
        "run1: connect+handshake {first_connect:?}, uploaded={}, facts: iface={} release={} arch={} host={}",
        conn.uploaded_agent,
        host.facts.default_ipv4_interface,
        host.facts.distribution_release,
        host.facts.architecture,
        host.facts.hostname,
    );
    assert!(!host.facts.architecture.is_empty());
    assert!(!host.facts.distribution_release.is_empty(), "Debian target");
    assert!(!host.facts.default_ipv4_interface.is_empty());
    conn.shutdown().await.expect("clean shutdown 1");

    // Event plumbing round-trip: M2 agents answer Plan with a Warn log.
    let (mut conn_ev, _) = ruxel_cli::transport::connect(&dest, &bin, "gate-run-events", false)
        .await
        .expect("event-test connect");
    conn_ev
        .send(&ruxel_proto::v1::Envelope {
            msg: Some(ruxel_proto::v1::envelope::Msg::Plan(
                ruxel_proto::v1::Plan::default(),
            )),
        })
        .await
        .expect("send plan");
    let event = conn_ev
        .next_event()
        .await
        .expect("event read")
        .expect("an event");
    assert!(
        matches!(
            event.msg,
            Some(ruxel_proto::v1::event::Msg::Log(ref l)) if l.message.contains("M3")
        ),
        "expected the M2 not-implemented log, got {event:?}"
    );
    conn_ev.shutdown().await.expect("clean shutdown events");

    // Run 2: same hash → no upload; warm master → fast.
    let t1 = Instant::now();
    let (conn2, _host2) = ruxel_cli::transport::connect(&dest, &bin, "gate-run-2", false)
        .await
        .expect("second connect");
    let second_connect = t1.elapsed();
    eprintln!(
        "run2: connect+handshake {second_connect:?}, uploaded={}",
        conn2.uploaded_agent
    );
    assert!(
        !conn2.uploaded_agent,
        "agent must not re-upload at the same hash"
    );
    conn2.shutdown().await.expect("clean shutdown 2");

    assert!(
        second_connect.as_secs_f64() < 1.0,
        "post-provision connect+handshake must be < 1 s, was {second_connect:?}"
    );
}
