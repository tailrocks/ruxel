use super::{Backend, Context, ContextLayer, Map, Result, Runner, TrackedValue, Value};
use std::collections::BTreeMap;

impl<B: Backend> Runner<'_, B> {
    pub(super) fn assertion_params(
        &mut self,
        params: &Map<String, Value>,
        scope: &TrackedValue,
        layer: &ContextLayer,
        mut sensitive: bool,
    ) -> Result<TrackedValue> {
        let raw = params.get("that").context("assert requires that")?;
        let entries = if let Some(items) = raw.as_array() {
            Some(items.as_slice())
        } else if raw.get("kind").and_then(Value::as_str) == Some("list") {
            Some(
                raw.get("items")
                    .and_then(Value::as_array)
                    .context("invalid assertion list")?
                    .as_slice(),
            )
        } else {
            None
        };
        let mut pass = true;
        if let Some(entries) = entries {
            for entry in entries {
                let evaluated = self.evaluate_scoped(entry, scope, layer).map_err(|error| {
                    if sensitive {
                        anyhow::anyhow!("sensitive assertion failed")
                    } else {
                        error
                    }
                })?;
                sensitive |= evaluated.is_secret();
                if !evaluated
                    .value
                    .as_bool()
                    .context("assertion must be boolean")?
                {
                    pass = false;
                    break;
                }
            }
        } else {
            let evaluated = self.evaluate_scoped(raw, scope, layer).map_err(|error| {
                if sensitive {
                    anyhow::anyhow!("sensitive assertion failed")
                } else {
                    error
                }
            })?;
            sensitive |= evaluated.is_secret();
            pass = match &evaluated.value {
                Value::Bool(value) => *value,
                Value::Array(items) => {
                    let mut pass = true;
                    for item in items {
                        pass &= item.as_bool().context("assertion must be boolean")?;
                    }
                    pass
                }
                _ => anyhow::bail!("assertion must be boolean or a list of booleans"),
            };
        }
        let mut fields = BTreeMap::new();
        fields.insert(
            "that".into(),
            if sensitive {
                TrackedValue::secret(Value::Bool(pass))
            } else {
                TrackedValue::public(Value::Bool(pass))
            },
        );
        for (key, value) in params {
            if key == "that" || (key == "fail_msg" && pass) {
                continue;
            }
            let value = self.evaluate_scoped(value, scope, layer).map_err(|error| {
                if sensitive {
                    anyhow::anyhow!("sensitive assertion failed")
                } else {
                    error
                }
            })?;
            sensitive |= value.is_secret();
            fields.insert(key.clone(), value);
        }
        Ok(TrackedValue::object(fields))
    }
}
