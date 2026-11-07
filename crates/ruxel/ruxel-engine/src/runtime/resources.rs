use crate::controller::Request;
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

#[derive(Default)]
pub(super) struct Resources {
    paths: BTreeSet<String>,
    unknown_effects: bool,
}

fn contains_path(value: &Value, paths: &BTreeSet<String>) -> bool {
    match value {
        Value::String(candidate) => paths.iter().any(|tracked| {
            let Some(a) = comparable_path(candidate) else {
                return true;
            };
            let Some(b) = comparable_path(tracked) else {
                return true;
            };
            a.starts_with(b) || b.starts_with(a)
        }),
        Value::Array(values) => values.iter().any(|v| contains_path(v, paths)),
        Value::Object(values) => values.values().any(|v| contains_path(v, paths)),
        _ => false,
    }
}

fn comparable_path(value: &str) -> Option<&Path> {
    let path = Path::new(value);
    // Remote cwd and symlink identities are unavailable here. Collapsing `..`
    // locally could select the wrong path when a preceding component is a link.
    (path.is_absolute() && !path.components().any(|part| part == Component::ParentDir))
        .then_some(path)
}

impl Resources {
    pub(super) fn mark_unknown(&mut self) {
        self.unknown_effects = true;
    }

    pub(super) fn has_sensitive_effects(&self) -> bool {
        self.unknown_effects || !self.paths.is_empty()
    }

    pub(super) fn observe(&mut self, request: &Request, input_sensitive: bool) -> Result<bool> {
        let mut params = Vec::new();
        for iteration in &request.task.iterations {
            params.push(
                serde_json::from_slice::<Value>(&iteration.params_json)
                    .context("invalid operation parameters")?,
            );
        }
        let opaque_request = !matches!(
            request.task.module.as_str(),
            "copy" | "template" | "file" | "slurp" | "stat"
        );
        let sensitive = input_sensitive
            || self.unknown_effects
            || (opaque_request && !self.paths.is_empty())
            || params.iter().any(|v| contains_path(v, &self.paths));
        if sensitive {
            match request.task.module.as_str() {
                "copy" | "template" | "file" => {
                    let key = if request.task.module == "file" {
                        "path"
                    } else {
                        "dest"
                    };
                    for value in &params {
                        if let Some(path) = value.get(key).and_then(Value::as_str) {
                            if comparable_path(path).is_some() {
                                self.paths.insert(path.to_owned());
                            } else {
                                self.unknown_effects = true;
                            }
                        } else {
                            self.unknown_effects = true;
                        }
                    }
                }
                "slurp" | "stat" => {}
                _ => self.unknown_effects = true,
            }
        }
        Ok(sensitive)
    }
}

#[cfg(test)]
mod tests;
