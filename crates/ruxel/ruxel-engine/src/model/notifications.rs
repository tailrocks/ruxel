use super::{Context, Result, Value, ensure};
use std::collections::BTreeSet;

pub(crate) fn validate(config: &Value) -> Result<()> {
    for play in array(config, "plays") {
        let handlers: BTreeSet<_> = array(play, "handlers")
            .iter()
            .filter_map(|handler| handler.get("name").and_then(Value::as_str))
            .collect();
        for phase in ["preTasks", "tasks", "handlers"] {
            validate_tasks(array(play, phase), &handlers)?;
        }
    }
    Ok(())
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn validate_tasks(tasks: &[Value], handlers: &BTreeSet<&str>) -> Result<()> {
    for task in tasks {
        for name in array(task, "notify") {
            let name = name
                .as_str()
                .context("notify entries must be literal handler names")?;
            ensure!(handlers.contains(name), "unknown notified handler: {name}");
        }
        for section in ["block", "rescue", "always"] {
            validate_tasks(array(task, section), handlers)?;
        }
    }
    Ok(())
}
