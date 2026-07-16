//! Stream framing: `varint length ‖ message bytes` over any Read/Write
//! (the agent's stdio on an SSH channel — docs/ARCHITECTURE.md §2). Sync
//! by design: the agent reads its plan single-threaded; the controller
//! wraps these on its async pipes by buffering whole frames.

use prost::Message;
use std::io::{self, Read, Write};

/// Upper bound on a single frame; anything larger is a protocol error,
/// not a real message (the biggest legitimate payloads — rendered task
/// params — are kilobytes).
pub const MAX_FRAME_LEN: u64 = 64 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct VarintDecoder {
    len: u64,
    shift: u32,
}

impl VarintDecoder {
    pub fn push(&mut self, byte: u8) -> Result<Option<u64>, &'static str> {
        self.len |= u64::from(byte & 0x7f) << self.shift;
        if byte & 0x80 == 0 {
            return Ok(Some(self.len));
        }
        self.shift += 7;
        if self.shift >= 64 {
            Err("frame length varint overflow")
        } else {
            Ok(None)
        }
    }
}

pub fn encode_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

pub fn encode_frame<M: Message>(msg: &M) -> Vec<u8> {
    let body = msg.encode_to_vec();
    let mut frame = Vec::with_capacity(body.len() + 10);
    encode_varint(body.len() as u64, &mut frame);
    frame.extend_from_slice(&body);
    frame
}

pub fn write_frame<M: Message>(w: &mut impl Write, msg: &M) -> io::Result<()> {
    let buf = encode_frame(msg);
    w.write_all(&buf)?;
    w.flush()
}

/// Read one frame; `Ok(None)` on clean EOF at a frame boundary.
pub fn read_frame<M: Message + Default>(r: &mut impl Read) -> io::Result<Option<M>> {
    let mut decoder = VarintDecoder::default();
    let mut first_byte = true;
    let len = loop {
        let mut byte = [0u8; 1];
        match r.read(&mut byte) {
            Ok(0) if first_byte => return Ok(None),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF inside frame length",
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
        first_byte = false;
        match decoder.push(byte[0]) {
            Ok(Some(len)) => break len,
            Ok(None) => {}
            Err(message) => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, message));
            }
        }
    };
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds MAX_FRAME_LEN"),
        ));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    M::decode(body.as_slice())
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1;
    use proptest::prelude::*;

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
        write_frame(&mut buf, &env).unwrap();
        write_frame(
            &mut buf,
            &v1::Envelope {
                msg: Some(v1::envelope::Msg::Done(v1::Done {})),
            },
        )
        .unwrap();

        let mut r = buf.as_slice();
        let first: v1::Envelope = read_frame(&mut r).unwrap().unwrap();
        assert!(matches!(first.msg, Some(v1::envelope::Msg::Hello(h)) if h == hello));
        let second: v1::Envelope = read_frame(&mut r).unwrap().unwrap();
        assert!(matches!(second.msg, Some(v1::envelope::Msg::Done(_))));
        let eof: Option<v1::Envelope> = read_frame(&mut r).unwrap();
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
        write_frame(&mut buf, &env).unwrap();
        buf.truncate(buf.len() - 3);
        let mut r = buf.as_slice();
        let res: io::Result<Option<v1::Envelope>> = read_frame(&mut r);
        assert!(res.is_err());
    }

    #[test]
    fn oversized_frame_is_rejected() {
        // A varint claiming 1 GiB.
        let mut buf = Vec::new();
        let mut len = 1u64 << 30;
        while len >= 0x80 {
            buf.push((len as u8 & 0x7f) | 0x80);
            len >>= 7;
        }
        buf.push(len as u8);
        let mut r = buf.as_slice();
        let res: io::Result<Option<v1::Envelope>> = read_frame(&mut r);
        assert!(res.is_err());
    }

    #[test]
    fn mid_varint_eof_is_error() {
        let mut input = [0x80].as_slice();
        let error = read_frame::<v1::Envelope>(&mut input).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn varint_overflow_is_error() {
        let mut input = [0x80; 10].as_slice();
        let error = read_frame::<v1::Envelope>(&mut input).unwrap_err();
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
        write_frame(&mut bytes, &expected).unwrap();
        let mut input = InterruptOnce {
            interrupted: false,
            bytes: io::Cursor::new(bytes),
        };
        assert_eq!(
            read_frame::<v1::Envelope>(&mut input).unwrap(),
            Some(expected)
        );
    }

    proptest! {
        #[test]
        fn read_frame_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let mut input = bytes.as_slice();
            let _ = read_frame::<v1::Envelope>(&mut input);
        }

        #[test]
        fn oversized_lengths_are_rejected_before_body_read(
            extra in 1_u64..(u32::MAX as u64)
        ) {
            let mut bytes = Vec::new();
            encode_varint(MAX_FRAME_LEN + extra, &mut bytes);
            let mut input = bytes.as_slice();
            let error = read_frame::<v1::Envelope>(&mut input).unwrap_err();
            prop_assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            prop_assert!(error.to_string().contains("MAX_FRAME_LEN"));
        }
    }
}
