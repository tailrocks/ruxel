use super::{Backend, ContextLayer, Result, Runner, TrackedValue, Value, array, json, string};

impl<B: Backend> Runner<'_, B> {
    pub(super) fn skip_unselected_task(
        &mut self,
        task: &Value,
        parent: &ContextLayer,
        layer: &ContextLayer,
    ) -> Result<bool> {
        if !self.handling
            && !self.options.tags.is_empty()
            && !super::super::tag_selection::selected(task, &parent.tags, &self.options.tags)
        {
            if array(task, "block").is_empty() {
                self.skip_tagged_leaf(task, layer)?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn skip_conditional_children(
        &mut self,
        tasks: &[Value],
        parent: &ContextLayer,
    ) -> Result<()> {
        for task in tasks {
            let layer = Self::layer(task, parent);
            if self.skip_unselected_task(task, parent, &layer)? {
                continue;
            }
            let children = array(task, "block");
            if !children.is_empty() {
                self.skip_conditional_children(children, &layer)?;
                continue;
            }
            self.recap.skipped += 1;
            let private = layer.no_log || layer.control_sensitive;
            if let Some(name) = string(task, "register") {
                let skipped = json!({"skipped":true,"changed":false,"failed":false,"skip":{"cause":"condition_false"}});
                let skipped = if private {
                    TrackedValue::secret(skipped)
                } else {
                    TrackedValue::public(skipped)
                };
                let mut results = self
                    .scope
                    .get("results")
                    .unwrap_or_else(|| TrackedValue::public(json!({})));
                results.insert(name.to_owned(), skipped)?;
                self.scope.insert("results".into(), results)?;
            }
            let result = if private {
                json!({"censored":true,"changed":false,"failed":false})
            } else {
                json!({"skipped":true,"changed":false})
            };
            self.backend.event(
                &json!({"host":self.host,"task":string(task,"name").unwrap_or(""),"result":result}),
            )?;
        }
        Ok(())
    }
}
