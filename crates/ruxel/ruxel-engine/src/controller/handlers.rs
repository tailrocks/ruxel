use super::{
    Backend, ContextLayer, Result, Runner, TrackedValue, Value, array, bail, flag, string,
};
use std::collections::BTreeSet;

impl<B: Backend> Runner<'_, B> {
    pub(super) fn queue_handlers(&mut self, task: &Value, result: &TrackedValue) {
        let sensitive = ["changed", "failed"]
            .iter()
            .any(|key| result.get(key).is_some_and(|value| value.is_secret()));
        for name in array(task, "notify").iter().filter_map(Value::as_str) {
            self.notified
                .entry(name.to_owned())
                .and_modify(|private| *private &= sensitive)
                .or_insert(sensitive);
        }
    }

    pub(super) async fn run_handlers(&mut self, layer: &ContextLayer) -> Result<()> {
        let handlers = self.handlers.clone();
        let defined: BTreeSet<_> = handlers
            .iter()
            .filter_map(|handler| string(handler, "name"))
            .collect();
        let mut executed = BTreeSet::new();
        for handler in &handlers {
            if let Some(name) = string(handler, "name")
                && !executed.contains(name)
                && let Some(sensitive) = self.notified.remove(name)
            {
                executed.insert(name);
                let mut context = layer.clone();
                context.control_sensitive |= sensitive;
                self.handling = true;
                let result = self.task(handler, &context).await;
                self.handling = false;
                let result = result.map_err(|error| {
                    if context.no_log || flag(handler, "noLog") || context.control_sensitive {
                        anyhow::anyhow!("sensitive task failed")
                    } else {
                        error
                    }
                })?;
                if !result {
                    break;
                }
            }
        }
        self.notified
            .retain(|name, _| !defined.contains(name.as_str()));
        if !self.notified.is_empty() {
            bail!(
                "unknown notified handlers: {:?}",
                self.notified.keys().collect::<Vec<_>>()
            );
        }
        Ok(())
    }
}
