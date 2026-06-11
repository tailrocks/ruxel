//! Controller↔agent protocol messages. See `proto/ruxel.proto` and
//! docs/ARCHITECTURE.md §2 for the framing model.

pub mod frame;

pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/ruxel.v1.rs"));
}

/// Protocol version spoken by this build. Bumped on incompatible changes;
/// the handshake rejects mismatches and triggers an agent re-upload.
pub const PROTO_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn hello_roundtrips() {
        let hello = v1::Hello {
            proto_version: PROTO_VERSION,
            run_id: "test-run".into(),
            check_mode: true,
            diff_mode: false,
            no_cache: false,
        };
        let bytes = hello.encode_to_vec();
        let decoded = v1::Hello::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded, hello);
    }
}
