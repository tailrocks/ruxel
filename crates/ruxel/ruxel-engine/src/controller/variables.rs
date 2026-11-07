use super::{Backend, ContextLayer, Map, Result, Runner, TrackedValue, Value};

impl<B: Backend> Runner<'_, B> {
    pub(super) fn sensitive_layered_condition(
        &self,
        value: &Value,
        scope: &TrackedValue,
        layer: &ContextLayer,
    ) -> bool {
        if super::operation::sensitive_condition(value, scope) {
            return true;
        }
        let definitions = self.variable_layers(layer);
        let names: Map<_, _> = definitions
            .iter()
            .flat_map(|vars| vars.keys())
            .map(|name| (name.clone(), Value::Null))
            .collect();
        let mut pending: Vec<_> = super::loop_vars::required(&names, value)
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
            let definition = &definitions[index][&name];
            if super::operation::sensitive_condition(definition, scope) {
                return true;
            }
            for dependency in super::loop_vars::required(&names, definition) {
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
