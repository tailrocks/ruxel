use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

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
    /// # Errors
    /// Returns an error if the backend cannot fetch the item.
    fn onepassword_item(&self, vault: &str, item: &str) -> Result<serde_json::Value, String>;
    /// # Errors
    /// Returns an error if the complete secret reference cannot be read.
    fn onepassword_reference(&self, reference: &str) -> Result<String, String>;
    /// # Errors
    /// Returns an error if the command fails.
    fn command(&self, argv: &[String]) -> Result<String, String>;
}

#[derive(Debug)]
pub struct ProcessBackend {
    fnox: PathBuf,
    op: PathBuf,
}

impl Default for ProcessBackend {
    fn default() -> Self {
        Self::with_programs("fnox", "op")
    }
}

impl ProcessBackend {
    #[must_use]
    pub fn with_programs(fnox: impl Into<PathBuf>, op: impl Into<PathBuf>) -> Self {
        Self {
            fnox: fnox.into(),
            op: op.into(),
        }
    }
}

fn process_output(mut command: Command) -> Result<String, String> {
    let result = command
        .output()
        .map_err(|_| "process could not start".to_owned())?;
    if !result.status.success() {
        return Err("process failed".into());
    }
    Ok(String::from_utf8_lossy(&result.stdout)
        .trim_end_matches('\n')
        .to_owned())
}

impl Backend for ProcessBackend {
    fn fnox(&self, key: &str) -> Result<String, String> {
        if key.is_empty()
            || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || key.as_bytes()[0].is_ascii_digit()
        {
            return Err("invalid environment key".into());
        }
        let mut cmd = Command::new(&self.fnox);
        cmd.args([
            "exec",
            "--no-redact",
            "--only",
            key,
            "--",
            "sh",
            "-c",
            "printenv -- \"$1\"",
            "sh",
            key,
        ]);
        process_output(cmd)
    }
    fn onepassword_item(&self, vault: &str, item: &str) -> Result<serde_json::Value, String> {
        let mut cmd = Command::new(&self.op);
        cmd.args([
            "item", "get", "--vault", vault, "--format", "json", "--", item,
        ]);
        serde_json::from_str(&process_output(cmd)?).map_err(|_| "invalid item response".into())
    }
    fn onepassword_reference(&self, reference: &str) -> Result<String, String> {
        let mut cmd = Command::new(&self.op);
        cmd.args(["read", "--", reference]);
        process_output(cmd)
    }
    fn command(&self, argv: &[String]) -> Result<String, String> {
        let (program, arguments) = argv.split_first().ok_or("empty command")?;
        let mut cmd = Command::new(program);
        cmd.args(arguments);
        process_output(cmd)
    }
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
type ItemCache = Arc<Mutex<HashMap<(String, String), Cell<serde_json::Value>>>>;

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
        let cell = self
            .items
            .lock()
            .map_err(|_| "lookup cache unavailable")?
            .entry((vault.to_owned(), item.to_owned()))
            .or_default()
            .clone();
        let document = cached(&cell, || {
            self.backend
                .onepassword_item(vault, item)
                .map_err(|_| "secret lookup failed".to_owned())
        })?;
        select_field(&document, field, section).ok_or_else(|| "secret field not found".to_owned())
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

fn select_field(
    document: &serde_json::Value,
    field: &str,
    section: Option<&str>,
) -> Option<String> {
    document
        .get("fields")?
        .as_array()?
        .iter()
        .find_map(|candidate| {
            let matches = |v: &serde_json::Value, wanted| {
                v.get("label").and_then(|v| v.as_str()) == Some(wanted)
                    || v.get("id").and_then(|v| v.as_str()) == Some(wanted)
            };
            if matches(candidate, field)
                && section.is_none_or(|s| candidate.get("section").is_some_and(|v| matches(v, s)))
            {
                candidate.get("value")?.as_str().map(str::to_owned)
            } else {
                None
            }
        })
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions need successful fake lookups"
)]
mod tests;
