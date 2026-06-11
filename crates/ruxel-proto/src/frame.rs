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

pub fn write_frame<M: Message>(w: &mut impl Write, msg: &M) -> io::Result<()> {
    let mut buf = Vec::with_capacity(msg.encoded_len() + 5);
    msg.encode_length_delimited(&mut buf)
        .expect("Vec<u8> write is infallible");
    w.write_all(&buf)?;
    w.flush()
}

/// Read one frame; `Ok(None)` on clean EOF at a frame boundary.
pub fn read_frame<M: Message + Default>(r: &mut impl Read) -> io::Result<Option<M>> {
    let mut len: u64 = 0;
    let mut shift = 0u32;
    let mut first_byte = true;
    loop {
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
        len |= u64::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame length varint overflow",
            ));
        }
    }
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
}
