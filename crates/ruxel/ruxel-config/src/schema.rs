//! The canonical Pkl schema, compiled into the binary and served to Pkl
//! code as `modulepath:/ruxel/Ruxel.pkl`.
//!
//! Configuration never copies the schema: the binary that evaluates a
//! configuration also supplies the schema it was built with. The served
//! schema carries a fingerprint of its own text in `Config.schemaFingerprint`,
//! so a configuration evaluated against any other copy, such as an old
//! `ruxel schema export`, is refused before it reaches the engine.

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

static SERVED: LazyLock<(String, String)> = LazyLock::new(|| {
    let fingerprint = format!("blake3:{}", blake3::hash(SOURCE.as_bytes()).to_hex());
    let source = SOURCE.replacen(PLACEHOLDER, &format!("\"{fingerprint}\""), 1);
    (fingerprint, source)
});

/// The fingerprint of the schema this binary serves.
#[must_use]
pub fn fingerprint() -> &'static str {
    &SERVED.0
}

/// The schema this binary serves at [`MODULE_URI`].
#[must_use]
pub fn source() -> &'static str {
    &SERVED.1
}

/// Check that an evaluated configuration was built against the served
/// schema, then remove the fingerprint so the engine sees only configuration.
///
/// The configuration is the root object or its `config` property.
///
/// # Errors
/// Returns [`ConfigError::Schema`] when the fingerprint is missing or differs.
pub fn admit(value: &mut serde_json::Value) -> Result<(), ConfigError> {
    let config = match value.get("config") {
        Some(_) => &mut value["config"],
        None => value,
    };
    let found = config
        .as_object_mut()
        .and_then(|config| config.remove(FINGERPRINT_PROPERTY));
    match found {
        Some(serde_json::Value::String(found)) if found == fingerprint() => Ok(()),
        found => Err(ConfigError::Schema {
            found: found.map(|found| {
                found
                    .as_str()
                    .map_or_else(|| found.to_string(), str::to_owned)
            }),
            expected: fingerprint().to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests;
