pub mod frame;

pub mod constants {
    pub const STATE_ROOT: &str = "/var/lib/ruxel";
    pub const AGENT_DIR: &str = "/var/lib/ruxel/agent";
    pub const DEFAULT_SYSCTL_FILE: &str = "/etc/sysctl.conf";
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

/// The protocol version the handshake enforces on both sides: v4
/// carries the workflow namespace that scopes ledger pruning and the
/// filtered completion bit whose `false` authorizes it. A v3 peer
/// decodes an empty Done as `filtered = false` and would prune the
/// records of tasks a tag-filtered or controller-aborted run never
/// sent — so mixed binaries are refused instead of silently applying
/// legacy completion semantics.
pub const PROTO_VERSION: u32 = 4;
