use crate::controller::Request;
use anyhow::{Context, Result};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

/// Conservative, per-destination record of sensitive external side effects.
#[derive(Default)]
pub(super) struct Resources {
    paths: BTreeSet<String>,
    unknown_effects: bool,
}

fn contains_path(value: &Value, paths: &BTreeSet<String>) -> bool {
    match value {
        Value::String(candidate) => paths.iter().any(|tracked| {
            let a = Path::new(candidate);
            let b = Path::new(tracked);
            a.starts_with(b) || b.starts_with(a)
        }),
        Value::Array(values) => values.iter().any(|v| contains_path(v, paths)),
        Value::Object(values) => values.values().any(|v| contains_path(v, paths)),
        _ => false,
    }
}

impl Resources {
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
                            self.paths.insert(path.to_owned());
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
mod tests {
    use super::*;
    use ruxel_proto::v1;
    use serde_json::json;

    fn request(module: &str, params: Value) -> Result<Request> {
        Ok(Request {
            host: "fixture".into(),
            task: v1::RenderedTask {
                module: module.into(),
                iterations: vec![v1::Iteration {
                    params_json: serde_json::to_vec(&params)?,
                    ..Default::default()
                }],
                ..Default::default()
            },
        })
    }

    #[test]
    fn tracks_sensitive_paths_and_unknown_side_effects_per_destination() -> Result<()> {
        let mut resources = Resources::default();
        assert!(resources.observe(
            &request("copy", json!({"dest":"/tmp/fixture-private"}))?,
            true
        )?);
        assert!(!resources.observe(
            &request("slurp", json!({"src":"/tmp/fixture-public"}))?,
            false
        )?);
        assert!(resources.observe(
            &request("slurp", json!({"src":"/tmp/fixture-private"}))?,
            false
        )?);
        assert!(resources.observe(
            &request("shell", json!({"cmd":"cat /tmp/fixture-private"}))?,
            false
        )?);
        assert!(resources.observe(&request("shell", json!({"cmd":"synthetic"}))?, true)?);
        assert!(resources.observe(&request("slurp", json!({"src":"/tmp/any"}))?, false)?);
        let mut other_destination = Resources::default();
        assert!(!other_destination.observe(&request("slurp", json!({"src":"/tmp/any"}))?, false)?);
        Ok(())
    }
}
