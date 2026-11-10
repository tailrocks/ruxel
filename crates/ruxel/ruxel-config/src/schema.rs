//! The canonical Pkl schema, compiled into the binary and served to Pkl
//! code as `modulepath:/ruxel/Ruxel.pkl`.
//!
//! Configuration never copies the schema: the binary that evaluates a
//! configuration also supplies the schema it was built with. The served
//! schema sets `Config.schemaFingerprint` to `blake3:<hash>@<nonce>`: the
//! BLAKE3 hash of `schema/Ruxel.pkl` and a nonce drawn once per process. Only
//! the module this process serves can produce the nonce, so admission refuses
//! every copy, even an unmodified one. `ruxel schema export` writes the
//! schema with the hash alone, for editors and the `pkl` CLI.

use crate::ConfigError;
use std::sync::LazyLock;

/// The Pkl module URI of the schema.
pub const MODULE_URI: &str = "modulepath:/ruxel/Ruxel.pkl";
/// The schema's path inside the Pkl module path.
pub const MODULE_PATH: &str = "/ruxel/Ruxel.pkl";
/// The `Config` property that carries the fingerprint.
pub const FINGERPRINT_PROPERTY: &str = "schemaFingerprint";

/// The schema exactly as committed in `schema/Ruxel.pkl`.
const SOURCE: &str = include_str!("../../../../schema/Ruxel.pkl");
/// The committed placeholder that the served schema replaces with the
/// fingerprint.
const PLACEHOLDER: &str = "\"unserved\"";

static FINGERPRINT: LazyLock<String> =
    LazyLock::new(|| format!("blake3:{}", blake3::hash(SOURCE.as_bytes()).to_hex()));
static SERVED_FINGERPRINT: LazyLock<String> =
    LazyLock::new(|| format!("{}@{}", fingerprint(), nonce()));
static SERVED: LazyLock<String> = LazyLock::new(|| with_fingerprint(&SERVED_FINGERPRINT));
static EXPORTED: LazyLock<String> = LazyLock::new(|| with_fingerprint(fingerprint()));

fn with_fingerprint(fingerprint: &str) -> String {
    SOURCE.replacen(PLACEHOLDER, &format!("\"{fingerprint}\""), 1)
}

/// 128 unpredictable bits, so no file can anticipate this process's nonce.
fn nonce() -> String {
    let mut seed = [0_u8; 16];
    let read = std::fs::File::open("/dev/urandom")
        .and_then(|mut random| std::io::Read::read_exact(&mut random, &mut seed));
    let mut hasher = blake3::Hasher::new();
    hasher.update(&seed);
    if read.is_err() {
        // Without the kernel source, mix process-unique values instead.
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        hasher.update(&time.to_le_bytes());
        hasher.update(&std::process::id().to_le_bytes());
        hasher.update(&(std::ptr::from_ref(&seed) as usize).to_le_bytes());
    }
    hasher.finalize().to_hex()[..32].to_owned()
}

/// The fingerprint of the schema this binary serves: `blake3:<hash>`.
#[must_use]
pub fn fingerprint() -> &'static str {
    &FINGERPRINT
}

/// The schema this process serves at [`MODULE_URI`].
#[must_use]
pub fn source() -> &'static str {
    &SERVED
}

/// The schema for editors and the `pkl` CLI, without this process's nonce.
#[must_use]
pub fn exported_source() -> &'static str {
    &EXPORTED
}

/// What a configuration's fingerprint says about the schema it used.
fn found_schema(found: Option<&serde_json::Value>) -> Result<(), ConfigError> {
    let Some(found) = found else {
        return Err(ConfigError::Schema {
            found: None,
            copy: false,
            expected: fingerprint().to_owned(),
        });
    };
    let text = found.as_str().unwrap_or_default();
    if text == SERVED_FINGERPRINT.as_str() {
        return Ok(());
    }
    let hash = text.split_once('@').map_or(text, |(hash, _)| hash);
    let recognized = hash == "unserved"
        || hash
            .strip_prefix("blake3:")
            .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()));
    Err(ConfigError::Schema {
        // Configuration chose this value; never echo an unrecognized one.
        found: Some(if recognized {
            hash.to_owned()
        } else {
            "an unrecognized fingerprint".to_owned()
        }),
        copy: hash == fingerprint(),
        expected: fingerprint().to_owned(),
    })
}

/// Check that an evaluated configuration was built against the schema this
/// process serves, then remove the fingerprint so the engine sees only
/// configuration.
///
/// The configuration is the root object or its `config` property.
///
/// # Errors
/// Returns [`ConfigError::Schema`] when the fingerprint is missing or is not
/// this process's.
pub fn admit(value: &mut serde_json::Value) -> Result<(), ConfigError> {
    let config = match value.get("config") {
        Some(_) => &mut value["config"],
        None => value,
    };
    let found = config
        .as_object_mut()
        .and_then(|config| config.remove(FINGERPRINT_PROPERTY));
    found_schema(found.as_ref())
}

#[cfg(test)]
mod tests;
