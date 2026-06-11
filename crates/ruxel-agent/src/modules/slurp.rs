//! `slurp` (SEMANTICS §6): read-only, returns base64 `content`.

use super::{params_object, str_param};
use serde_json::{Value, json};

pub fn run(params: &Value) -> Result<Value, String> {
    let obj = params_object(params)?;
    let src = str_param(obj, "src").ok_or("slurp: src required")?;
    let bytes = std::fs::read(src).map_err(|e| format!("slurp {src}: {e}"))?;
    Ok(json!({
        "content": b64(&bytes),
        "source": src,
        "encoding": "base64",
        "changed": false,
        "failed": false,
    }))
}

/// Standard base64 (the agent stays dependency-light; 15 lines beats a
/// crate for one call site).
fn b64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn b64_matches_python() {
        // python3: base64.b64encode(b"26.2.1\n") == b"MjYuMi4xCg=="
        assert_eq!(super::b64(b"26.2.1\n"), "MjYuMi4xCg==");
        assert_eq!(super::b64(b""), "");
        assert_eq!(super::b64(b"a"), "YQ==");
        assert_eq!(super::b64(b"ab"), "YWI=");
    }
}
