//! The Ruxel Pkl modules, compiled into the binary and served to Pkl code
//! under `modulepath:/ruxel/`: the workflow language
//! (`modulepath:/ruxel/Workflow.pkl`, `modulepath:/ruxel/Ruxel.pkl`) and the
//! engine schema (`modulepath:/ruxel/engine/Engine.pkl`).
//!
//! Configuration never copies a module: the binary that evaluates a
//! configuration also supplies the modules it was built with. The served
//! engine schema sets `Config.schemaFingerprint` to `blake3:<hash>@<nonce>`:
//! the BLAKE3 hash of every module path and text under `schema/ruxel/`, and a
//! nonce drawn once per process. The two language modules that build a
//! `Config`, `Workflow.pkl` and `internal/Lowering.pkl`, carry the same value
//! and record it as `Config.languageFingerprint`. Only the modules this
//! process serves can produce the nonce, so admission refuses every copy of
//! any of them, even an unmodified one. `ruxel schema export` writes the
//! modules with the hash alone, for editors and the `pkl` CLI.

use crate::ConfigError;
use std::sync::LazyLock;

/// Where configuration imports Ruxel from: every module is served under this
/// `modulepath:` namespace.
pub const MODULE_PREFIX: &str = "modulepath:/ruxel/";
/// The engine schema, which configurations written against the engine
/// directly import.
pub const ENGINE_URI: &str = "modulepath:/ruxel/engine/Engine.pkl";
/// The `Config` property that carries the fingerprint.
pub const FINGERPRINT_PROPERTY: &str = "schemaFingerprint";

/// The modules exactly as committed under `schema/`, by their path inside
/// the Pkl module path.
pub(crate) const TREE: &[(&str, &str)] = &[
    (
        "/ruxel/Ruxel.pkl",
        include_str!("../../../../schema/ruxel/Ruxel.pkl"),
    ),
    (
        "/ruxel/Workflow.pkl",
        include_str!("../../../../schema/ruxel/Workflow.pkl"),
    ),
    (
        "/ruxel/internal/Lowering.pkl",
        include_str!("../../../../schema/ruxel/internal/Lowering.pkl"),
    ),
    (
        "/ruxel/engine/Engine.pkl",
        include_str!("../../../../schema/ruxel/engine/Engine.pkl"),
    ),
];
/// The module that declares `Config.schemaFingerprint`.
pub(crate) const ENGINE: &str = "/ruxel/engine/Engine.pkl";
/// The modules whose first [`PLACEHOLDER`] the served and exported texts
/// replace with the fingerprint: the engine, which sets
/// `Config.schemaFingerprint`, and the two language modules that together set
/// `Config.languageFingerprint`, so a copy of either is refused too.
pub(crate) const FINGERPRINTED: &[&str] = &[
    ENGINE,
    "/ruxel/Workflow.pkl",
    "/ruxel/internal/Lowering.pkl",
];
/// The committed placeholder that the served and exported modules replace
/// with the fingerprint.
pub(crate) const PLACEHOLDER: &str = "\"unserved\"";
/// The `Config` property the workflow language sets.
pub const LANGUAGE_FINGERPRINT_PROPERTY: &str = "languageFingerprint";

static FINGERPRINT: LazyLock<String> = LazyLock::new(|| {
    let mut hasher = blake3::Hasher::new();
    for (path, text) in TREE {
        hasher
            .update(path.as_bytes())
            .update(&[0])
            .update(text.as_bytes())
            .update(&[0]);
    }
    format!("blake3:{}", hasher.finalize().to_hex())
});
static SERVED_FINGERPRINT: LazyLock<String> =
    LazyLock::new(|| format!("{}@{}", fingerprint(), nonce()));
static SERVED: LazyLock<Vec<(&'static str, String)>> =
    LazyLock::new(|| with_fingerprint(&SERVED_FINGERPRINT));
static EXPORTED: LazyLock<Vec<(&'static str, String)>> =
    LazyLock::new(|| with_fingerprint(fingerprint()));

fn with_fingerprint(fingerprint: &str) -> Vec<(&'static str, String)> {
    TREE.iter()
        .map(|(path, text)| {
            let text = if FINGERPRINTED.contains(path) {
                text.replacen(PLACEHOLDER, &format!("\"{fingerprint}\""), 1)
            } else {
                (*text).to_owned()
            };
            (*path, text)
        })
        .collect()
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

/// The fingerprint of the modules this binary serves: `blake3:<hash>`.
#[must_use]
pub fn fingerprint() -> &'static str {
    &FINGERPRINT
}

/// The modules this process serves, by module-path path.
#[must_use]
pub fn modules() -> &'static [(&'static str, String)] {
    &SERVED
}

/// The modules for editors and the `pkl` CLI, without this process's nonce.
#[must_use]
pub fn exported_modules() -> &'static [(&'static str, String)] {
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
    let object = config.as_object_mut();
    let (found, language) = match object {
        Some(config) => (
            config.remove(FINGERPRINT_PROPERTY),
            config.remove(LANGUAGE_FINGERPRINT_PROPERTY),
        ),
        None => (None, None),
    };
    found_schema(found.as_ref())?;
    // Built by the workflow language: its modules must be this process's too.
    match language {
        None | Some(serde_json::Value::Null) => Ok(()),
        Some(language) => found_schema(Some(&language)),
    }
}

#[cfg(test)]
mod tests;
