//! Controller↔agent protocol messages. See `proto/ruxel.proto` and
//! docs/ARCHITECTURE.md §2 for the framing model.

pub mod frame;

pub mod constants {
    pub const STATE_ROOT: &str = "/var/lib/ruxel";
    pub const AGENT_DIR: &str = "/var/lib/ruxel/agent";
    pub const DEFAULT_SYSCTL_FILE: &str = "/etc/sysctl.conf";
    /// Controller `HelloAck` deadline and agent orphan guard stay coupled.
    pub const HANDSHAKE_TIMEOUT_SECS: u64 = 30;
}

#[expect(
    clippy::doc_markdown,
    clippy::must_use_candidate,
    clippy::struct_excessive_bools,
    reason = "prost-generated code"
)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/ruxel.v1.rs"));
}

/// Protocol version spoken by this build. Bumped on incompatible changes;
/// the handshake rejects mismatches and triggers an agent re-upload.
pub const PROTO_VERSION: u32 = 1;
