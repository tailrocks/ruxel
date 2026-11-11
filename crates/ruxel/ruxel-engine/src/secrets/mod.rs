mod backend;
mod fingerprint;
mod generation;
mod lock;
mod lookup;
mod resolver;
mod staging;
mod store;

pub use backend::ProcessBackend;
pub(crate) use fingerprint::{is_fingerprint, is_sentence_over};
pub use generation::{GeneratedRequest, generate_value, generated_requests};
pub use lock::FnoxLock;
pub use lookup::Lookup;
pub use resolver::Resolver;

use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

/// The one fnox key grammar, enforced by the Core schema's `FnoxKey`, the
/// Engine schema's `Lookup.key` constraint, model validation and the backend
/// alike. Core and Lowering reuse this sentence in their messages.
pub(crate) const FNOX_KEY_GRAMMAR: &str =
    "uppercase letters, digits and underscores only, starting with a letter, 2 to 128 characters";

/// The formats generation creates a missing key in. `Core.pkl` spells the
/// same union as `GeneratedFormat`; there is no kubeadm token format — the
/// kubeadm recipe composes the pair from two fnox keys.
pub(crate) const GENERATED_FORMATS: [&str; 4] = ["hex16", "hex32", "base64-32", "alnum32"];

/// Whether `key` is an `FnoxKey`: `[A-Z][A-Z0-9_]{1,127}`.
pub(crate) fn is_fnox_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    (2..=128).contains(&bytes.len())
        && bytes[0].is_ascii_uppercase()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
}

/// Whether `format` is a `GeneratedFormat`.
pub(crate) fn is_generated_format(format: &str) -> bool {
    GENERATED_FORMATS.contains(&format)
}

pub trait Backend: Send + Sync {
    /// Reads one fnox key. `Ok(None)` means the key is missing; an `Err`
    /// names the failure redacted, never the value.
    ///
    /// # Errors
    /// Returns an error if the backend cannot resolve the key.
    fn fnox_get(&self, key: &str) -> Result<Option<String>, String>;
    /// Stores one fnox key with the value staged in a file — `fnox set KEY
    /// --from-file`, the byte-preserving store — never in a command
    /// argument and never on stdin, which the store trims.
    ///
    /// # Errors
    /// Returns an error if the value cannot be stored.
    fn fnox_set(&self, key: &str, value: &str) -> Result<(), String>;
    /// Holds the store's write lock while generation checks presence,
    /// generates, stores and reads back, while a `FetchSecret` store
    /// compares, writes and invalidates the cache, and while a lookup
    /// resolves a cached read of the key — reads and writes of one key
    /// serialize on it. Runs that honor the lock serialize; the guard
    /// releases on drop.
    ///
    /// # Errors
    /// Returns an error if the lock cannot be taken.
    fn fnox_write_lock(&self, _key: &str) -> Result<FnoxLock, String> {
        FnoxLock::unheld()
    }

    /// Fetches only the named field of the item: the field objects whose
    /// label or id matches `field` — case-insensitively, as 1Password matches
    /// labels — never the whole item. When a section is named, the backend
    /// uses the 1Password CLI's `section.field` selector.
    ///
    /// # Errors
    /// Returns an error if the backend cannot fetch the field.
    fn onepassword_fields(
        &self,
        vault: &str,
        item: &str,
        section: Option<&str>,
        field: &str,
    ) -> Result<serde_json::Value, String>;
    /// # Errors
    /// Returns an error if the complete secret reference cannot be read.
    fn onepassword_reference(&self, reference: &str) -> Result<String, String>;
    /// # Errors
    /// Returns an error if the command fails.
    fn command(&self, argv: &[String]) -> Result<String, String>;
}

pub(super) type Cell<T> = Arc<Mutex<Option<T>>>;

/// The selector-derived placeholder a lookup answers in synthetic mode:
/// `plan`, `check` and `diff` show this, and the keyed fingerprint shown
/// beside a changed secret-bearing file derives from it. It derives from the
/// selector, never the value, so synthetic mode is rotation-blind by design
/// (a rotated value is never detected there); the design states that limit
/// rather than promising rotation detection it does not have.
#[must_use]
pub(crate) fn selector_placeholder(lookup: &Lookup) -> String {
    synthetic_value(lookup)
}

pub(super) fn parse_op_reference(reference: &str) -> Option<(&str, &str, Option<&str>, &str)> {
    let parts: Vec<_> = reference.strip_prefix("op://")?.split('/').collect();
    match parts.as_slice() {
        [vault, item, field] => Some((vault, item, None, field)),
        [vault, item, section, field] => Some((vault, item, Some(section), field)),
        _ => None,
    }
}

pub(super) fn synthetic_value(lookup: &Lookup) -> String {
    let (kind, parts): (&str, Vec<&str>) = match lookup {
        Lookup::Fnox { key } => ("fnox", vec![key]),
        Lookup::FnoxGenerated { key, format } => ("fnox-generated", vec![key, format]),
        Lookup::OnePassword {
            vault,
            item,
            field,
            section,
        } => (
            "onepassword",
            vec![item, field, vault, section.as_deref().unwrap_or("")],
        ),
        Lookup::OnePasswordUri { reference } => ("op-uri", vec![reference]),
        Lookup::Command { argv } => ("command", argv.iter().map(String::as_str).collect()),
        Lookup::Synthetic { value } => return value.clone(),
    };
    let mut key = kind.to_owned();
    for part in parts {
        key.push('\u{1f}');
        key.push_str(part);
    }
    let digest = Sha256::digest(key.as_bytes());
    let mut hex = String::with_capacity(16);
    for byte in &digest[..8] {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
        hex.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    format!("dry-secret-{hex}")
}

/// Selects the one field a lookup names. Several fields may share a label
/// across sections: without a section the lookup fails naming the sections,
/// never taking the first match; with one, the fields of that section alone
/// are considered. 1Password matches labels case-insensitively — a reference
/// `op://Vault/Item/PASSWORD` names the field labeled `password` — so labels
/// and ids fold case here too, and two labels that differ only by case are
/// two matches, not one. The message names labels, never values.
pub(super) fn select_field(
    document: &serde_json::Value,
    item: &str,
    field: &str,
    section: Option<&str>,
) -> Result<String, String> {
    let matches = |v: &serde_json::Value, wanted: &str| {
        ["label", "id"].iter().any(|key| {
            v.get(*key)
                .and_then(|v| v.as_str())
                .is_some_and(|have| have.to_lowercase() == wanted.to_lowercase())
        })
    };
    let candidates: Vec<&serde_json::Value> = document
        .as_array()
        .map(|fields| fields.iter().collect())
        .unwrap_or_default();
    let hits: Vec<&serde_json::Value> = candidates
        .iter()
        .filter(|candidate| {
            matches(candidate, field)
                && section.is_none_or(|s| candidate.get("section").is_some_and(|v| matches(v, s)))
        })
        .copied()
        .collect();
    if hits.len() > 1 {
        // With no section on any match there is no section to name that
        // could disambiguate, so the labels themselves are named.
        if hits.iter().all(|hit| hit.get("section").is_none()) {
            let labels: Vec<String> = hits
                .iter()
                .map(|hit| {
                    hit.get("label")
                        .or_else(|| hit.get("id"))
                        .and_then(|v| v.as_str())
                        .map_or_else(|| "no label".to_owned(), |name| format!("\"{name}\""))
                })
                .collect();
            return Err(format!(
                "ambiguous 1Password field \"{field}\" in item \"{item}\": it matches labels {}",
                labels.join(", ")
            ));
        }
        let sections: Vec<String> = hits
            .iter()
            .map(|hit| {
                hit.get("section")
                    .and_then(|s| {
                        s.get("label")
                            .and_then(|v| v.as_str())
                            .or_else(|| s.get("id").and_then(|v| v.as_str()))
                    })
                    .map_or_else(|| "no section".to_owned(), |name| format!("\"{name}\""))
            })
            .collect();
        return Err(format!(
            "ambiguous 1Password field \"{field}\" in item \"{item}\": it matches sections {}; name the section",
            sections.join(", ")
        ));
    }
    hits.first()
        .and_then(|hit| hit.get("value"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| "secret field not found".to_owned())
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions need successful fake lookups"
)]
mod tests;
