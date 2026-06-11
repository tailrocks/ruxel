//! Ruxel agent: runs on the target host, executes compiled plans streamed
//! over stdio. M0 skeleton — the protocol loop, module runtime, and ledger
//! land in M2/M3 (docs/PLAN.md).

use ruxel_proto::PROTO_VERSION;

fn main() {
    // Placeholder entrypoint: prove the cross-compiled binary runs on the
    // target and reports its identity. Replaced by the framed-protocol loop
    // in M2.
    println!(
        "ruxel-agent {} (proto v{})",
        env!("CARGO_PKG_VERSION"),
        PROTO_VERSION
    );
}
