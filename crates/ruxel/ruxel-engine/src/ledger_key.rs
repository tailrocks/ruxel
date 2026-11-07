use crate::secret::Secret;
use serde_json::Value;
use std::fmt;

#[derive(Debug, Eq, PartialEq)]
pub enum LedgerKeyError {
    MissingSecret(String),
    Canonicalization,
}

impl fmt::Display for LedgerKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSecret(name) => write!(f, "missing secret {name}"),
            Self::Canonicalization => f.write_str("could not canonicalize ledger parameters"),
        }
    }
}
impl std::error::Error for LedgerKeyError {}

/// Computes a stable, secret-aware key for a resource's ledger entry.
///
/// # Errors
/// Returns [`LedgerKeyError::MissingSecret`] when a referenced secret is absent,
/// or [`LedgerKeyError::Canonicalization`] if parameters cannot be encoded.
pub fn ledger_key<'a>(
    fleet: &str,
    host: &str,
    id: &str,
    type_tag: &str,
    params: &Value,
    secret: &'a dyn Fn(&str) -> Option<&'a Secret>,
) -> Result<String, LedgerKeyError> {
    let canonical = canonical(params, secret)?;
    let input = [
        fleet,
        host,
        id,
        type_tag,
        &canonical,
        &ruxel_proto::PROTO_VERSION.to_string(),
    ]
    .join("\0");
    Ok(hex(&blake3::derive_key(
        "ruxel ledger v2",
        input.as_bytes(),
    )))
}

fn canonical<'a>(
    value: &Value,
    secret: &'a dyn Fn(&str) -> Option<&'a Secret>,
) -> Result<String, LedgerKeyError> {
    if let Some(object) = value.as_object() {
        if object.get("kind").and_then(Value::as_str) == Some("secretRef") {
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let value =
                secret(name).ok_or_else(|| LedgerKeyError::MissingSecret(name.to_owned()))?;
            let mut payload = name.as_bytes().to_vec();
            payload.push(0);
            payload.extend_from_slice(value.expose().as_bytes());
            return serde_json::to_string(&format!(
                "secret:{name}:{}",
                hex(&blake3::derive_key("ruxel secret fingerprint v1", &payload))
            ))
            .map_err(|_| LedgerKeyError::Canonicalization);
        }
        let mut keys: Vec<_> = object.keys().collect();
        keys.sort();
        let mut out = String::from("{");
        for (index, key) in keys.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(
                &serde_json::to_string(*key).map_err(|_| LedgerKeyError::Canonicalization)?,
            );
            out.push(':');
            out.push_str(&canonical(&object[*key], secret)?);
        }
        out.push('}');
        return Ok(out);
    }
    if let Some(array) = value.as_array() {
        let mut out = String::from("[");
        for (index, item) in array.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&canonical(item, secret)?);
        }
        out.push(']');
        return Ok(out);
    }
    serde_json::to_string(value).map_err(|_| LedgerKeyError::Canonicalization)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(test)]
#[path = "ledger_key/tests.rs"]
mod tests;
