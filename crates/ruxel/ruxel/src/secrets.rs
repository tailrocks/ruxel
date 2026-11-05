//! Run-scoped 1Password resolver. Item JSON is fetched once and serves every
//! field/section lookup from that item; apply prefetches discovered items on a
//! bounded worker pool before compilation.

use ruxel_core::engine::LookupResolver;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ItemRef {
    pub vault: String,
    pub item: String,
}

pub trait ItemFetcher: Send + Sync {
    fn fetch(&self, item: &ItemRef) -> Result<serde_json::Value, String>;
}

#[derive(Default)]
pub struct CliItemFetcher;

impl ItemFetcher for CliItemFetcher {
    fn fetch(&self, item: &ItemRef) -> Result<serde_json::Value, String> {
        let output = std::process::Command::new("op")
            .args([
                "item",
                "get",
                &item.item,
                "--vault",
                &item.vault,
                "--format",
                "json",
            ])
            .output()
            .map_err(|error| format!("spawn op: {error} (is the 1Password CLI installed?)"))?;
        if !output.status.success() {
            return Err(format!(
                "op item get failed for item {:?} in vault {:?}: {}",
                item.item,
                item.vault,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("op item JSON for {:?}: {error}", item.item))
    }
}

#[derive(Clone)]
pub struct OpResolver {
    fetcher: Arc<dyn ItemFetcher>,
    items: Arc<Mutex<HashMap<ItemRef, serde_json::Value>>>,
}

impl Default for OpResolver {
    fn default() -> Self {
        Self::new(Arc::new(CliItemFetcher))
    }
}

impl OpResolver {
    pub fn new(fetcher: Arc<dyn ItemFetcher>) -> Self {
        Self {
            fetcher,
            items: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn item_json(&self, item: &ItemRef) -> Result<serde_json::Value, String> {
        if let Some(value) = self.items.lock().unwrap().get(item).cloned() {
            return Ok(value);
        }
        let value = self.fetcher.fetch(item)?;
        self.items
            .lock()
            .unwrap()
            .insert(item.clone(), value.clone());
        Ok(value)
    }

    pub fn prefetch(&self, items: impl IntoIterator<Item = ItemRef>) -> Result<(), String> {
        let queue = Arc::new(Mutex::new(
            items
                .into_iter()
                .filter(|item| !self.items.lock().unwrap().contains_key(item))
                .collect::<Vec<_>>(),
        ));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let workers = queue.lock().unwrap().len().min(8);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let queue = Arc::clone(&queue);
                let errors = Arc::clone(&errors);
                scope.spawn(move || {
                    loop {
                        let Some(item) = queue.lock().unwrap().pop() else {
                            break;
                        };
                        if let Err(error) = self.item_json(&item) {
                            errors.lock().unwrap().push(error);
                        }
                    }
                });
            }
        });
        let mut errors = errors.lock().unwrap();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.remove(0))
        }
    }
}

impl LookupResolver for OpResolver {
    fn onepassword(
        &self,
        item: &str,
        field: Option<&str>,
        vault: Option<&str>,
        section: Option<&str>,
    ) -> Result<String, String> {
        let item_ref = ItemRef {
            vault: vault.unwrap_or("Private").to_string(),
            item: item.to_string(),
        };
        let document = self.item_json(&item_ref)?;
        select_field(&document, field.unwrap_or("password"), section).ok_or_else(|| {
            format!(
                "onepassword(item={item:?}, field={:?}, vault={:?}): field not found",
                field.unwrap_or("password"),
                item_ref.vault
            )
        })
    }

    fn pipe(&self, cmd: &str) -> Result<String, String> {
        if let Some(reference) = pipe_op_reference(cmd)
            && let Some((vault, item, section, field)) = parse_op_reference(reference)
        {
            return self.onepassword(&item, Some(&field), Some(&vault), section.as_deref());
        }
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .map_err(|error| format!("pipe spawn: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "pipe command failed (rc={:?}): {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end_matches('\n')
            .to_string())
    }
}

pub fn discover_items(source: &str) -> BTreeSet<ItemRef> {
    let mut items = BTreeSet::new();
    for (start, _) in source.match_indices("op://") {
        let tail = &source[start..];
        let delimiter = source[..start]
            .chars()
            .next_back()
            .filter(|character| *character == '\'' || *character == '"');
        let end = delimiter
            .and_then(|quote| tail.find(quote))
            .or_else(|| tail.find(|character: char| character.is_whitespace()))
            .unwrap_or(tail.len());
        if let Some((vault, item, _, _)) = parse_op_reference(&tail[..end]) {
            items.insert(ItemRef { vault, item });
        }
    }
    // community.general.onepassword calls need expression-aware discovery;
    // scan each lookup expression, then extract positional item and vault kwarg.
    for expression in source.match_indices("community.general.onepassword") {
        let tail = &source[expression.0..source.len().min(expression.0 + 512)];
        let Some(end) = tail.find("}}") else { continue };
        let call = &tail[..end];
        let quoted = quoted_values(call.split_once(',').map_or(call, |(_, args)| args));
        let Some(item) = quoted.first() else { continue };
        let vault = keyword_quoted(call, "vault").unwrap_or_else(|| "Private".into());
        items.insert(ItemRef {
            vault,
            item: item.clone(),
        });
    }
    items
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
            let field_matches = candidate.get("label").and_then(|value| value.as_str())
                == Some(field)
                || candidate.get("id").and_then(|value| value.as_str()) == Some(field);
            let section_matches = section.is_none_or(|wanted| {
                candidate.get("section").is_some_and(|actual| {
                    actual.get("label").and_then(|value| value.as_str()) == Some(wanted)
                        || actual.get("id").and_then(|value| value.as_str()) == Some(wanted)
                })
            });
            (field_matches && section_matches)
                .then(|| candidate.get("value")?.as_str().map(str::to_string))?
        })
}

fn pipe_op_reference(cmd: &str) -> Option<&str> {
    let start = cmd.find("op://")?;
    let rest = &cmd[start..];
    let delimiter = cmd[..start]
        .chars()
        .next_back()
        .filter(|character| *character == '\'' || *character == '"');
    let end = delimiter
        .and_then(|quote| rest.find(quote))
        .or_else(|| rest.find(|character: char| character.is_whitespace()))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

fn parse_op_reference(reference: &str) -> Option<(String, String, Option<String>, String)> {
    let parts: Vec<_> = reference.strip_prefix("op://")?.split('/').collect();
    match parts.as_slice() {
        [vault, item, field] => Some(((*vault).into(), (*item).into(), None, (*field).into())),
        [vault, item, section, field] => Some((
            (*vault).into(),
            (*item).into(),
            Some((*section).into()),
            (*field).into(),
        )),
        _ => None,
    }
}

fn quoted_values(input: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\'' || character == '"' {
            let mut value = String::new();
            for next in chars.by_ref() {
                if next == character {
                    break;
                }
                value.push(next);
            }
            values.push(value);
        }
    }
    values
}

fn keyword_quoted(input: &str, keyword: &str) -> Option<String> {
    let start = input.find(keyword)? + keyword.len();
    let tail = input[start..].trim_start();
    let tail = tail.strip_prefix('=')?.trim_start();
    quoted_values(tail).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeFetcher(AtomicUsize);

    impl ItemFetcher for FakeFetcher {
        fn fetch(&self, _item: &ItemRef) -> Result<serde_json::Value, String> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(serde_json::json!({"fields": [
                {"id": "username", "label": "username", "value": "alice"},
                {"id": "password", "label": "password", "value": "secret"}
            ]}))
        }
    }

    #[test]
    fn multiple_fields_fetch_item_once() {
        let fetcher = Arc::new(FakeFetcher(AtomicUsize::new(0)));
        let resolver = OpResolver::new(fetcher.clone());
        assert_eq!(
            resolver
                .onepassword("db", Some("username"), Some("V"), None)
                .unwrap(),
            "alice"
        );
        assert_eq!(
            resolver
                .onepassword("db", Some("password"), Some("V"), None)
                .unwrap(),
            "secret"
        );
        assert_eq!(
            resolver.pipe("op read 'op://V/db/username'").unwrap(),
            "alice"
        );
        assert_eq!(fetcher.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn discovers_plugin_and_pipe_items() {
        let source = r#"{{ lookup('community.general.onepassword', 'db', field='password', vault='V') }}
{{ lookup('pipe', "op read 'op://V/ssh item/private key'") }}"#;
        let found = discover_items(source);
        assert!(found.contains(&ItemRef {
            vault: "V".into(),
            item: "db".into()
        }));
        assert!(found.contains(&ItemRef {
            vault: "V".into(),
            item: "ssh item".into()
        }));
    }
}
