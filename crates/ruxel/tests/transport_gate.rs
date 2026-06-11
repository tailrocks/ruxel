//! The M2 transport gate, one connect per process (see the known-issue
//! note in transport.rs): cold connect → handshake → facts → clean
//! shutdown. tools/fixtures/gate.sh drives it twice — the second run
//! asserts the content-addressed upload was skipped and the warm
//! connect+handshake stays under the 1 s budget.
//!
//!   RUXEL_TEST_SSH_DEST=<dest> RUXEL_TEST_AGENT_BIN=<agent> \
//!   [RUXEL_TEST_SSH_KEY=<keyfile>] \
//!   [RUXEL_TEST_EXPECT_NO_UPLOAD=1] [RUXEL_TEST_EXPECT_FAST=1] \
//!   cargo test -p ruxel-cli --test transport_gate -- --ignored --nocapture

use std::time::Instant;

fn env_or_skip() -> Option<(String, std::path::PathBuf)> {
    let dest = std::env::var("RUXEL_TEST_SSH_DEST").ok()?;
    let bin = std::env::var("RUXEL_TEST_AGENT_BIN").ok()?;
    Some((dest, bin.into()))
}

fn options() -> ruxel_cli::transport::ConnectOptions {
    ruxel_cli::transport::ConnectOptions {
        keyfile: std::env::var("RUXEL_TEST_SSH_KEY").ok().map(Into::into),
        accept_new_host_key: std::env::var("RUXEL_TEST_SSH_KEY").is_ok(),
        known_hosts_file: std::env::var("RUXEL_TEST_SSH_KEY")
            .ok()
            .map(|k| format!("{k}.known_hosts").into()),
        diff_mode: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs RUXEL_TEST_SSH_DEST + RUXEL_TEST_AGENT_BIN (fixture VM)"]
async fn connect_handshake_facts_shutdown() {
    let Some((dest, bin)) = env_or_skip() else {
        eprintln!("env not set — skipped");
        return;
    };

    let t0 = Instant::now();
    let (conn, host) = ruxel_cli::transport::connect_with(&dest, &bin, "gate", false, &options())
        .await
        .expect("connect");
    let elapsed = t0.elapsed();
    eprintln!(
        "connect+handshake {elapsed:?}, uploaded={}, facts: iface={} release={} arch={} host={}",
        conn.uploaded_agent,
        host.facts.default_ipv4_interface,
        host.facts.distribution_release,
        host.facts.architecture,
        host.facts.hostname,
    );
    assert!(!host.facts.architecture.is_empty());
    assert!(!host.facts.distribution_release.is_empty(), "Debian target");
    assert!(!host.facts.default_ipv4_interface.is_empty());

    if std::env::var("RUXEL_TEST_EXPECT_NO_UPLOAD").is_ok() {
        assert!(
            !conn.uploaded_agent,
            "agent must not re-upload at the same hash"
        );
    }
    conn.shutdown().await.expect("clean shutdown");

    if std::env::var("RUXEL_TEST_EXPECT_FAST").is_ok() {
        assert!(
            elapsed.as_secs_f64() < 1.0,
            "post-provision connect+handshake must be < 1 s, was {elapsed:?}"
        );
    }
}
