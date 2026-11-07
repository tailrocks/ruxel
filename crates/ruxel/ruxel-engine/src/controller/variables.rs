use super::{Backend, ContextLayer, Map, Result, Runner, TrackedValue, Value};

fn known_public_bool(
    value: &Value,
    scope: &TrackedValue,
    definitions: &[Map<String, Value>],
) -> Option<bool> {
    match value.get("kind")?.as_str()? {
        "literal" => value.get("value")?.as_bool(),
        "ref" => {
            let path = value.get("path")?.as_array()?;
            let root = path.first()?.as_str()?;
            if matches!(root, "vars" | "variables")
                && path
                    .get(1)
                    .and_then(Value::as_str)
                    .is_some_and(|name| definitions.iter().any(|layer| layer.contains_key(name)))
            {
                return None;
            }
            let mut current = scope.clone();
            for (index, part) in path.iter().enumerate() {
                let part = part.as_str()?;
                current = current.get(if index == 0 && part == "variables" {
                    "vars"
                } else {
                    part
                })?;
            }
            if current.is_secret() {
                None
            } else {
                current.value.as_bool()
            }
        }
        _ => None,
    }
}

fn prune_known_branches(
    value: &Value,
    scope: &TrackedValue,
    definitions: &[Map<String, Value>],
) -> Value {
    match value {
        Value::Object(fields) => {
            if fields.get("kind").and_then(Value::as_str) == Some("call")
                && let Some(args) = fields.get("args").and_then(Value::as_array)
                && let Some(first) = args.first()
                && let Some(condition) = known_public_bool(first, scope, definitions)
            {
                match (
                    fields.get("function").and_then(Value::as_str),
                    args.len(),
                    condition,
                ) {
                    (Some("if"), 3, true) => {
                        return prune_known_branches(&args[1], scope, definitions);
                    }
                    (Some("if"), 3, false) => {
                        return prune_known_branches(&args[2], scope, definitions);
                    }
                    (Some("and"), 2, false) | (Some("or"), 2, true) => return first.clone(),
                    _ => {}
                }
            }
            Value::Object(
                fields
                    .iter()
                    .map(|(key, item)| {
                        (key.clone(), prune_known_branches(item, scope, definitions))
                    })
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| prune_known_branches(item, scope, definitions))
                .collect(),
        ),
        _ => value.clone(),
    }
}

impl<B: Backend> Runner<'_, B> {
    pub(super) fn sensitive_layered_condition(
        &self,
        value: &Value,
        scope: &TrackedValue,
        layer: &ContextLayer,
    ) -> bool {
        let definitions = self.variable_layers(layer);
        let value = prune_known_branches(value, scope, &definitions);
        if super::operation::sensitive_condition(&value, scope) {
            return true;
        }
        let names: Map<_, _> = definitions
            .iter()
            .flat_map(|vars| vars.keys())
            .map(|name| (name.clone(), Value::Null))
            .collect();
        let mut pending: Vec<_> = super::loop_vars::required(&names, &value)
            .into_iter()
            .map(|name| (definitions.len(), name))
            .collect();
        let mut visited = std::collections::BTreeSet::new();
        while let Some((ceiling, name)) = pending.pop() {
            let Some(index) = (0..ceiling)
                .rev()
                .find(|index| definitions[*index].contains_key(&name))
            else {
                continue;
            };
            if !visited.insert((index, name.clone())) {
                continue;
            }
            let definition = prune_known_branches(&definitions[index][&name], scope, &definitions);
            if super::operation::sensitive_condition(&definition, scope) {
                return true;
            }
            for dependency in super::loop_vars::required(&names, &definition) {
                let ceiling = if dependency == name {
                    index
                } else {
                    definitions.len()
                };
                pending.push((ceiling, dependency));
            }
        }
        false
    }

    pub(super) fn variable_layers(&self, layer: &ContextLayer) -> Vec<Map<String, Value>> {
        let facts = self.scope.get("setFacts");
        layer
            .definitions
            .iter()
            .map(|definitions| {
                definitions
                    .iter()
                    .filter(|(name, _)| {
                        facts
                            .as_ref()
                            .is_none_or(|facts| facts.value.get(*name).is_none())
                    })
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect::<Map<_, _>>()
            })
            .filter(|layer| !layer.is_empty())
            .collect()
    }

    pub(super) fn evaluate_scoped(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        layer: &ContextLayer,
    ) -> Result<TrackedValue> {
        let definitions = self.variable_layers(layer);
        self.backend
            .evaluate_layered_expression(value, scope, &definitions)
            .map(|(result, _)| result)
    }
}
