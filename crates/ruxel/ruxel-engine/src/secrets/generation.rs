//! Creating a missing generated key: the formats, one request per key,
//! and the collection of a configuration's generated lookups.

use super::fingerprint::random_bytes;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GeneratedRequest {
    pub key: String,
    pub format: String,
}

/// A missing generated key is created in its named format: 16 or 32
/// lowercase hex characters, 32 bytes base64-encoded, or 32 letters and
/// digits. There is no kubeadm token format: the kubeadm recipe composes
/// the pair from two fnox keys.
///
/// # Errors
/// Returns an error if the operating system's random source is unavailable.
pub fn generate_value(format: &str) -> Result<String, String> {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let random = |bytes: usize| -> Result<Vec<u8>, String> {
        let mut buffer = vec![0_u8; bytes];
        random_bytes(&mut buffer)?;
        Ok(buffer)
    };
    let hex = |bytes: Vec<u8>| -> String {
        let mut value = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            value.push(char::from(DIGITS[usize::from(byte >> 4)]));
            value.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
        }
        value
    };
    match format {
        "hex16" => Ok(hex(random(8)?)),
        "hex32" => Ok(hex(random(16)?)),
        "base64-32" => {
            use base64::Engine as _;
            Ok(base64::engine::general_purpose::STANDARD.encode(random(32)?))
        }
        "alnum32" => {
            const ALPHABET: &[u8; 62] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
            // Rejection sampling: a byte at least 248 would bias the
            // alphabet's last letters, so it is redrawn.
            let mut value = Vec::with_capacity(32);
            while value.len() < 32 {
                for byte in random(64)? {
                    if byte < 248 {
                        value.push(ALPHABET[usize::from(byte % 62)]);
                        if value.len() == 32 {
                            break;
                        }
                    }
                }
            }
            Ok(String::from_utf8(value).map_err(|_| "unknown generated format".to_owned())?)
        }
        _ => Err("unknown generated format".to_owned()),
    }
}

/// Collects every generated lookup of a rendered configuration, in field
/// order, deduplicated. Generation runs once per key for the whole run.
/// One key is created in one format: the same key referenced with two
/// formats is a configuration error, never a choice made by the order the
/// configuration is walked in.
///
/// # Errors
/// Returns an error naming the key and both formats, when one key is
/// referenced with two.
pub fn generated_requests(config: &serde_json::Value) -> Result<Vec<GeneratedRequest>, String> {
    fn walk(
        value: &serde_json::Value,
        requests: &mut Vec<GeneratedRequest>,
        formats: &mut Vec<(String, String)>,
    ) -> Result<(), String> {
        match value {
            serde_json::Value::Object(fields) => {
                let is_generated_lookup = fields.get("kind").and_then(serde_json::Value::as_str)
                    == Some("lookup")
                    && fields.get("provider").and_then(serde_json::Value::as_str)
                        == Some("fnox-generated");
                if is_generated_lookup
                    && let (Some(key), Some(format)) = (
                        fields.get("key").and_then(serde_json::Value::as_str),
                        fields.get("format").and_then(serde_json::Value::as_str),
                    )
                {
                    one_format(key, format, formats)?;
                    let request = GeneratedRequest {
                        key: key.to_owned(),
                        format: format.to_owned(),
                    };
                    if !requests.contains(&request) {
                        requests.push(request);
                    }
                }
                for child in fields.values() {
                    walk(child, requests, formats)?;
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    walk(child, requests, formats)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut requests = Vec::new();
    let mut formats = Vec::new();
    walk(config, &mut requests, &mut formats)?;
    Ok(requests)
}

/// One key is created in one format. The same key referenced with two
/// formats is a configuration error: the run names the key and both
/// formats, whichever order the configuration is walked in.
///
/// # Errors
/// Returns an error naming the key and both formats, when one key is
/// referenced with two.
pub(crate) fn one_format(
    key: &str,
    format: &str,
    seen: &mut Vec<(String, String)>,
) -> Result<(), String> {
    match seen.iter().find(|(seen_key, _)| seen_key.as_str() == key) {
        Some((_, first)) if first != format => {
            // The two formats are named in one order, whichever order the
            // configuration is walked in.
            let mut formats = [first.as_str(), format];
            formats.sort_unstable();
            Err(format!(
                "generated secret {key} is referenced with two formats: {} and {}",
                formats[0], formats[1]
            ))
        }
        Some(_) => Ok(()),
        None => {
            seen.push((key.to_owned(), format.to_owned()));
            Ok(())
        }
    }
}
