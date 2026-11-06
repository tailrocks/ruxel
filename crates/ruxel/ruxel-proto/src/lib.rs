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

pub const PROTO_VERSION: u32 = 2;
