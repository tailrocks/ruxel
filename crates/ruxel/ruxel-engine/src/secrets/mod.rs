//! Controller-side lookup resolution. Secret values never appear in diagnostics.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

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
    Pipe {
        command: String,
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
            Self::Pipe { .. } => "Pipe([REDACTED])",
            Self::Synthetic { .. } => "Synthetic([REDACTED])",
        })
    }
}

/// Inject a backend to run lookups. Backend errors are deliberately not surfaced.
pub trait Backend: Send + Sync {
    fn fnox(&self, key: &str) -> Result<String, String>;
    fn onepassword_item(&self, vault: &str, item: &str) -> Result<serde_json::Value, String>;
    fn command(&self, argv: &[String]) -> Result<String, String>;
    fn pipe(&self, command: &str) -> Result<String, String>;
}

#[derive(Default)]
pub struct ProcessBackend;

fn output(mut command: Command) -> Result<String, String> {
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
        let mut cmd = Command::new("fnox");
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
        output(cmd)
    }
    fn onepassword_item(&self, vault: &str, item: &str) -> Result<serde_json::Value, String> {
        let mut cmd = Command::new("op");
        cmd.args(["item", "get", item, "--vault", vault, "--format", "json"]);
        serde_json::from_str(&output(cmd)?).map_err(|_| "invalid item response".into())
    }
    fn command(&self, argv: &[String]) -> Result<String, String> {
        let (program, args) = argv.split_first().ok_or("empty command")?;
        let mut cmd = Command::new(program);
        cmd.args(args);
        output(cmd)
    }
    fn pipe(&self, command: &str) -> Result<String, String> {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);
        output(cmd)
    }
}

type Cell<T> = Arc<OnceLock<Result<T, String>>>;

#[derive(Clone)]
pub struct Resolver {
    backend: Arc<dyn Backend>,
    values: Arc<Mutex<HashMap<Lookup, Cell<String>>>>,
    items: Arc<Mutex<HashMap<(String, String), Cell<serde_json::Value>>>>,
}

impl Resolver {
    #[must_use]
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Self {
            backend,
            values: Arc::default(),
            items: Arc::default(),
        }
    }

    /// Resolve a lookup; concurrent requests for the same value share one fetch.
    pub fn resolve(&self, lookup: &Lookup) -> Result<String, String> {
        let cell = self
            .values
            .lock()
            .map_err(|_| "lookup cache unavailable")?
            .entry(lookup.clone())
            .or_default()
            .clone();
        cell.get_or_init(|| self.resolve_uncached(lookup)).clone()
    }

    fn resolve_uncached(&self, lookup: &Lookup) -> Result<String, String> {
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
                let (vault, item, section, field) =
                    parse_op_reference(reference).ok_or_else(generic)?;
                self.op_field(vault, item, field, section)
            }
            Lookup::Command { argv } => self.backend.command(argv).map_err(|_| generic()),
            Lookup::Pipe { command } => {
                if let Some(reference) = pipe_op_reference(command) {
                    if let Some((vault, item, section, field)) = parse_op_reference(reference) {
                        return self.op_field(vault, item, field, section);
                    }
                }
                self.backend.pipe(command).map_err(|_| generic())
            }
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
        let document = cell
            .get_or_init(|| {
                self.backend
                    .onepassword_item(vault, item)
                    .map_err(|_| "secret lookup failed".to_owned())
            })
            .as_ref()
            .map_err(Clone::clone)?;
        select_field(document, field, section).ok_or_else(|| "secret field not found".to_owned())
    }

    /// Fetch distinct lookups concurrently, with at most eight workers.
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
                        if self.resolve(&lookup).is_err() {
                            if let Ok(mut value) = failed.lock() {
                                *value = true;
                            }
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

fn pipe_op_reference(command: &str) -> Option<&str> {
    let start = command.find("op://")?;
    let tail = &command[start..];
    let quote = command[..start]
        .chars()
        .next_back()
        .filter(|c| *c == '\'' || *c == '"');
    let end = quote
        .and_then(|q| tail.find(q))
        .or_else(|| tail.find(char::is_whitespace))
        .unwrap_or(tail.len());
    Some(&tail[..end])
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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Fake {
        items: AtomicUsize,
        calls: AtomicUsize,
    }
    impl Backend for Fake {
        fn fnox(&self, _: &str) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok("fake-value".into())
        }
        fn onepassword_item(&self, _: &str, _: &str) -> Result<serde_json::Value, String> {
            self.items.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({"fields": [
                {"id":"username", "label":"Username", "value":"alice"},
                {"id":"password", "label":"Password", "section":{"id":"login", "label":"Login"}, "value":"fake-password"}
            ]}))
        }
        fn command(&self, _: &[String]) -> Result<String, String> {
            Err("fake-sensitive-error".into())
        }
        fn pipe(&self, _: &str) -> Result<String, String> {
            Ok("fake-pipe".into())
        }
    }

    #[test]
    fn prefetched_fields_share_item_and_select_ids_or_labels() {
        let backend = Arc::new(Fake::default());
        let resolver = Resolver::new(backend.clone());
        let user = Lookup::OnePasswordUri {
            reference: "op://vault/item/username".into(),
        };
        let password = Lookup::Pipe {
            command: "op read 'op://vault/item/Login/Password'".into(),
        };
        resolver
            .prefetch(&[user.clone(), password.clone(), user.clone()])
            .unwrap();
        assert_eq!(resolver.resolve(&user).unwrap(), "alice");
        assert_eq!(resolver.resolve(&password).unwrap(), "fake-password");
        assert_eq!(backend.items.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn other_lookup_types_cache_and_redact_errors() {
        let backend = Arc::new(Fake::default());
        let resolver = Resolver::new(backend.clone());
        let key = Lookup::Fnox { key: "KEY".into() };
        resolver.prefetch(&[key.clone(), key.clone()]).unwrap();
        assert_eq!(resolver.resolve(&key).unwrap(), "fake-value");
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
        let command = Lookup::Command {
            argv: vec!["fake-sensitive-command".into()],
        };
        assert_eq!(
            resolver.resolve(&command).unwrap_err(),
            "secret lookup failed"
        );
        assert!(!format!("{command:?}").contains("fake-sensitive"));
        assert_eq!(
            resolver
                .resolve(&Lookup::Synthetic {
                    value: "synthetic".into()
                })
                .unwrap(),
            "synthetic"
        );
        assert_eq!(
            resolver
                .resolve(&Lookup::Pipe {
                    command: "fake".into()
                })
                .unwrap(),
            "fake-pipe"
        );
    }
}
