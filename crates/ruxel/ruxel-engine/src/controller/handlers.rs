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

    /// Whether the handler flush ran to its end: a handler that
    /// stops the play without an execution error — `Ok(false)` —
    /// leaves notified handlers unexecuted, so the session stopped
    /// before its tasks were all sent and the recap must carry the
    /// aborted mark, the way a failed play task does. The caller
    /// maps `Ok(false)` onto that mark; the unexecuted notified
    /// handlers stay defined, so the unknown-handler bail below
    /// never fires for them on either path.
    pub(super) async fn run_handlers(&mut self, layer: &ContextLayer) -> Result<bool> {
        let handlers = self.handlers.clone();
        let defined: BTreeSet<_> = handlers
            .iter()
            .filter_map(|handler| string(handler, "name"))
            .collect();
        let mut executed = BTreeSet::new();
        let mut completed = true;
        'flush: loop {
            let before = executed.len();
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
                        completed = false;
                        break 'flush;
                    }
                }
            }
            if executed.len() == before {
                break;
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
        Ok(completed)
    }
}
