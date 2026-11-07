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
        let mut names = std::collections::BTreeSet::new();
        for variables in &definitions {
            names.extend(super::loop_vars::required(variables, value));
        }
        loop {
            let before = names.len();
            for variables in &definitions {
                for name in names.clone() {
                    if let Some(definition) = variables.get(&name) {
                        if super::operation::sensitive_condition(definition, scope) {
                            return true;
                        }
                        names.extend(super::loop_vars::required(variables, definition));
                    }
                }
            }
            if names.len() == before {
                return false;
            }
        }
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
