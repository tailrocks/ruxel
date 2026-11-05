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
    /// Add one byte to the length prefix decoder.
    ///
    /// Returns the decoded length when the prefix ends, or `None` while more
    /// bytes are needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the varint exceeds 64 bits.
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
        output.push((value.to_le_bytes()[0] & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value.to_le_bytes()[0]);
}

pub fn encode_frame<M: Message>(msg: &M) -> Vec<u8> {
    let body = msg.encode_to_vec();
    let mut frame = Vec::with_capacity(body.len() + 10);
    encode_varint(body.len() as u64, &mut frame);
    frame.extend_from_slice(&body);
    frame
}

/// Encode and write one complete frame, then flush the writer.
///
/// # Errors
///
/// Returns an error if writing or flushing the frame fails.
pub fn write_frame<M: Message>(w: &mut impl Write, msg: &M) -> io::Result<()> {
    let buf = encode_frame(msg);
    w.write_all(&buf)?;
    w.flush()
}

/// Read one frame; `Ok(None)` on clean EOF at a frame boundary.
///
/// # Errors
///
/// Returns an error for a truncated prefix or body, an invalid or oversized
/// length, or a message that cannot be decoded.
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
    let body_len = usize::try_from(len).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "frame length does not fit this platform",
        )
    })?;
    let mut body = vec![0u8; body_len];
    r.read_exact(&mut body)?;
    M::decode(body.as_slice())
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
