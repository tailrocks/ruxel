use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

mod backend;

pub use backend::ProcessBackend;

/// The one fnox key grammar, enforced by the Core schema's `FnoxKey`, the
/// Engine schema's `Lookup.key` constraint, model validation and the backend
/// alike. Core and Lowering reuse this sentence in their messages.
pub(crate) const FNOX_KEY_GRAMMAR: &str =
    "uppercase letters, digits and underscores only, starting with a letter, 2 to 128 characters";

/// Whether `key` is an `FnoxKey`: `[A-Z][A-Z0-9_]{1,127}`.
pub(crate) fn is_fnox_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    (2..=128).contains(&bytes.len())
        && bytes[0].is_ascii_uppercase()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub enum Lookup {
    Fnox {
        key: String,
    },
    OnePassword {
        vault: String,
        item: String,
        field: String,
        section: Option<String>,
    },
    OnePasswordUri {
        reference: String,
    },
    Command {
        argv: Vec<String>,
    },
    Synthetic {
        value: String,
    },
}

impl fmt::Debug for Lookup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Fnox { .. } => "Fnox([REDACTED])",
            Self::OnePassword { .. } | Self::OnePasswordUri { .. } => "OnePassword([REDACTED])",
            Self::Command { .. } => "Command([REDACTED])",
            Self::Synthetic { .. } => "Synthetic([REDACTED])",
        })
    }
}

pub trait Backend: Send + Sync {
    /// # Errors
    /// Returns an error if the backend cannot resolve the key.
    fn fnox(&self, key: &str) -> Result<String, String>;
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

type Cell<T> = Arc<Mutex<Option<T>>>;

fn cached<T: Clone>(
    cell: &Cell<T>,
    resolve: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let mut value = cell.lock().map_err(|_| "lookup cache unavailable")?;
    if let Some(value) = value.as_ref() {
        return Ok(value.clone());
    }
    let resolved = resolve()?;
    *value = Some(resolved.clone());
    Ok(resolved)
}
type ItemCache =
    Arc<Mutex<HashMap<(String, String, Option<String>, String), Cell<serde_json::Value>>>>;

#[derive(Clone)]
pub struct Resolver {
    backend: Arc<dyn Backend>,
    synthetic: bool,
    values: Arc<Mutex<HashMap<Lookup, Cell<String>>>>,
    items: ItemCache,
}

impl fmt::Debug for Resolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Resolver([REDACTED])")
    }
}

impl Resolver {
    #[must_use]
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Self {
            backend,
            synthetic: false,
            values: Arc::default(),
            items: Arc::default(),
        }
    }

    #[must_use]
    pub fn synthetic() -> Self {
        let mut resolver = Self::new(Arc::new(ProcessBackend::default()));
        resolver.synthetic = true;
        resolver
    }

    /// # Errors
    /// Returns a redacted error if the lookup fails.
    pub fn resolve(&self, lookup: &Lookup) -> Result<String, String> {
        let cell = self
            .values
            .lock()
            .map_err(|_| "lookup cache unavailable")?
            .entry(lookup.clone())
            .or_default()
            .clone();
        cached(&cell, || self.resolve_uncached(lookup))
    }

    fn resolve_uncached(&self, lookup: &Lookup) -> Result<String, String> {
        if self.synthetic {
            return Ok(synthetic_value(lookup));
        }
        let generic = || "secret lookup failed".to_owned();
        match lookup {
            Lookup::Fnox { key } => self.backend.fnox(key).map_err(|_| generic()),
            Lookup::OnePassword {
                vault,
                item,
                field,
                section,
            } => self.op_field(vault, item, field, section.as_deref()),
            Lookup::OnePasswordUri { reference } => {
                if reference.contains('?') {
                    return self
                        .backend
                        .onepassword_reference(reference)
                        .map_err(|_| generic());
                }
                let (vault, item, section, field) =
                    parse_op_reference(reference).ok_or_else(generic)?;
                self.op_field(vault, item, field, section)
            }
            Lookup::Command { argv } => self.backend.command(argv).map_err(|_| generic()),
            Lookup::Synthetic { value } => Ok(value.clone()),
        }
    }

    fn op_field(
        &self,
        vault: &str,
        item: &str,
        field: &str,
        section: Option<&str>,
    ) -> Result<String, String> {
        // A section and field reach `op item get --fields` as one selector.
        // The CLI parses `,` as a list separator, `=` as `label=`/`type=`
        // selectors and `.` as the section separator, so a legal component
        // like "API,key" or "API.Password" would fetch a different selector
        // than the author named. Authoring refuses the same characters
        // (Core's `onePassword`); this mirrors it, because a configuration
        // can name a selector directly.
        if let Some(section) = section
            && (section.contains(',') || section.contains('='))
        {
            return Err(format!(
                "1Password section \"{section}\" may not contain a comma or an equals sign: \
                 `op item get --fields` reads them as selectors, so it would fetch unintended fields"
            ));
        }
        if let Some(section) = section
            && section.contains('.')
        {
            return Err(format!(
                "1Password section \"{section}\" may not contain a dot: \
                 `op item get --fields` reads a dot as the section separator, so it would fetch a different selector than the author named"
            ));
        }
        if field.contains(',') || field.contains('=') {
            return Err(format!(
                "1Password field \"{field}\" may not contain a comma or an equals sign: \
                 `op item get --fields` reads them as selectors, so it would fetch unintended fields"
            ));
        }
        if field.contains('.') {
            return Err(format!(
                "1Password field \"{field}\" may not contain a dot: \
                 `op item get --fields` reads a dot as the section separator, so it would fetch a different selector than the author named"
            ));
        }
        // The fetch is field- and section-scoped, so the cache is too: the
        // same label in two sections is two distinct reads and never shares
        // a fetched document.
        let cell = self
            .items
            .lock()
            .map_err(|_| "lookup cache unavailable")?
            .entry((
                vault.to_owned(),
                item.to_owned(),
                section.map(str::to_owned),
                field.to_owned(),
            ))
            .or_default()
            .clone();
        let document = cached(&cell, || {
            self.backend
                .onepassword_fields(vault, item, section, field)
                .map_err(|_| "secret lookup failed".to_owned())
        })?;
        select_field(&document, item, field, section)
    }

    /// # Errors
    /// Returns a redacted error if any lookup fails.
    pub fn prefetch(&self, lookups: &[Lookup]) -> Result<(), String> {
        let unique: HashSet<_> = lookups.iter().cloned().collect();
        let queue = Mutex::new(unique.into_iter().collect::<Vec<_>>());
        let failed = Mutex::new(false);
        std::thread::scope(|scope| {
            for _ in 0..lookups.len().min(8) {
                scope.spawn(|| {
                    loop {
                        let next = queue.lock().ok().and_then(|mut q| q.pop());
                        let Some(lookup) = next else { break };
                        if self.resolve(&lookup).is_err()
                            && let Ok(mut value) = failed.lock()
                        {
                            *value = true;
                        }
                    }
                });
            }
        });
        if *failed.lock().map_err(|_| "lookup cache unavailable")? {
            Err("secret lookup failed".into())
        } else {
            Ok(())
        }
    }
}

fn parse_op_reference(reference: &str) -> Option<(&str, &str, Option<&str>, &str)> {
    let parts: Vec<_> = reference.strip_prefix("op://")?.split('/').collect();
    match parts.as_slice() {
        [vault, item, field] => Some((vault, item, None, field)),
        [vault, item, section, field] => Some((vault, item, Some(section), field)),
        _ => None,
    }
}

fn synthetic_value(lookup: &Lookup) -> String {
    let (kind, parts): (&str, Vec<&str>) = match lookup {
        Lookup::Fnox { key } => ("fnox", vec![key]),
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
fn select_field(
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
