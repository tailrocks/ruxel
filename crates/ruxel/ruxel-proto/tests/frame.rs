use proptest::prelude::*;
use prost::Message;
use ruxel_proto::{PROTO_VERSION, frame::*, v1};
use std::io::{self, Read};

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
    let decoded = v1::Hello::decode(bytes.as_slice()).expect("hello frame decodes");
    assert_eq!(decoded, hello);
}

#[test]
fn roundtrip_through_buffer() {
    let mut buf = Vec::new();
    let hello = v1::Hello {
        proto_version: 1,
        run_id: "r1".into(),
        ..Default::default()
    };
    let env = v1::Envelope {
        msg: Some(v1::envelope::Msg::Hello(hello.clone())),
    };
    write_frame(&mut buf, &env).expect("frame operation succeeds");
    write_frame(
        &mut buf,
        &v1::Envelope {
            msg: Some(v1::envelope::Msg::Done(v1::Done {})),
        },
    )
    .expect("frame operation succeeds");

    let mut r = buf.as_slice();
    let first: v1::Envelope = read_frame(&mut r)
        .expect("frame operation succeeds")
        .expect("frame operation succeeds");
    assert!(matches!(first.msg, Some(v1::envelope::Msg::Hello(h)) if h == hello));
    let second: v1::Envelope = read_frame(&mut r)
        .expect("frame operation succeeds")
        .expect("frame operation succeeds");
    assert!(matches!(second.msg, Some(v1::envelope::Msg::Done(_))));
    let eof: Option<v1::Envelope> = read_frame(&mut r).expect("frame operation succeeds");
    assert!(eof.is_none());
}

#[test]
fn truncated_body_is_an_error() {
    let mut buf = Vec::new();
    let env = v1::Envelope {
        msg: Some(v1::envelope::Msg::Hello(v1::Hello {
            proto_version: 1,
            run_id: "truncate-me".into(),
            ..Default::default()
        })),
    };
    write_frame(&mut buf, &env).expect("frame operation succeeds");
    buf.truncate(buf.len() - 3);
    let mut r = buf.as_slice();
    let res: io::Result<Option<v1::Envelope>> = read_frame(&mut r);
    assert!(res.is_err());
}

#[test]
fn oversized_frame_is_rejected() {
    let mut buf = Vec::new();
    let mut len = 1u64 << 30;
    while len >= 0x80 {
        buf.push((len.to_le_bytes()[0] & 0x7f) | 0x80);
        len >>= 7;
    }
    buf.push(len.to_le_bytes()[0]);
    let mut r = buf.as_slice();
    let res: io::Result<Option<v1::Envelope>> = read_frame(&mut r);
    assert!(res.is_err());
}

#[test]
fn mid_varint_eof_is_error() {
    let mut input = [0x80].as_slice();
    let error = read_frame::<v1::Envelope>(&mut input).expect_err("invalid frame returns an error");
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn varint_overflow_is_error() {
    let mut input = [0x80; 10].as_slice();
    let error = read_frame::<v1::Envelope>(&mut input).expect_err("invalid frame returns an error");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("varint overflow"));
}

#[test]
fn interrupted_read_retries() {
    struct InterruptOnce {
        interrupted: bool,
        bytes: io::Cursor<Vec<u8>>,
    }
    impl Read for InterruptOnce {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            self.bytes.read(buf)
        }
    }

    let expected = v1::Envelope {
        msg: Some(v1::envelope::Msg::Done(v1::Done {})),
    };
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &expected).expect("frame operation succeeds");
    let mut input = InterruptOnce {
        interrupted: false,
        bytes: io::Cursor::new(bytes),
    };
    assert_eq!(
        read_frame::<v1::Envelope>(&mut input).expect("frame operation succeeds"),
        Some(expected)
    );
}

proptest! {
    #[test]
    fn read_frame_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let mut input = bytes.as_slice();
        drop(read_frame::<v1::Envelope>(&mut input));
    }

    #[test]
    fn oversized_lengths_are_rejected_before_body_read(
        extra in 1_u64..u64::from(u32::MAX)
    ) {
        let mut bytes = Vec::new();
        encode_varint(MAX_FRAME_LEN + extra, &mut bytes);
        let mut input = bytes.as_slice();
        let error = read_frame::<v1::Envelope>(&mut input).expect_err("invalid frame returns an error");
        prop_assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        prop_assert!(error.to_string().contains("MAX_FRAME_LEN"));
    }
}
