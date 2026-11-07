use super::{Backend, Result, Value, array, flag, json, string};

fn redact_lookups(value: &mut Value) {
    match value {
        Value::Object(fields) if fields.get("kind").and_then(Value::as_str) == Some("lookup") => {
            *value = json!({"kind":"lookup","censored":true});
        }
        Value::Object(fields) => {
            for child in fields.values_mut() {
                redact_lookups(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                redact_lookups(child);
            }
        }
        _ => {}
    }
}

fn visible(task: &Value) -> Value {
    if flag(task, "noLog") {
        return json!({"name":string(task,"name"),"noLog":true,"censored":true});
    }
    let mut output = task.clone();
    for section in ["block", "rescue", "always"] {
        if let Some(children) = output.get_mut(section).and_then(Value::as_array_mut) {
            for child in children {
                *child = visible(child);
            }
        }
    }
    redact_lookups(&mut output);
    output
}

fn project(task: &Value, filter: &[String], inherited: &[String]) -> Option<(Value, u64)> {
    let mut tags = inherited.to_vec();
    tags.extend(
        array(task, "tags")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned),
    );
    if !filter.is_empty() && !super::tag_selection::selected(task, inherited, filter) {
        return None;
    }
    let mut output = task.clone();
    let mut count = u64::from(task.get("operation").is_some_and(|value| !value.is_null()));
    for section in ["block", "rescue", "always"] {
        if let Some(children) = output.get_mut(section).and_then(Value::as_array_mut) {
            let mut selected = Vec::new();
            for child in children.iter() {
                if let Some((value, child_count)) = project(child, filter, &tags) {
                    count += child_count;
                    selected.push(value);
                }
            }
            *children = selected;
        }
    }
    Some((visible(&output), count))
}

pub(super) fn plan_tasks<B: Backend>(
    backend: &mut B,
    host: &str,
    tasks: &[Value],
    phase: &str,
    filter: &[String],
    inherited: &[String],
) -> Result<u64> {
    let mut count = 0;
    let visible_host = if backend.conservative_sensitive() {
        "[censored]"
    } else {
        host
    };
    for task in tasks {
        if let Some((selected, task_count)) = project(task, filter, inherited) {
            let selected = if backend.conservative_sensitive() {
                json!({"censored":true})
            } else {
                selected
            };
            backend.event(
                &json!({"host":visible_host,"phase":phase,"planned":true,"task":selected}),
            )?;
            count += task_count;
        }
    }
    Ok(count)
}

pub(super) fn plan_handlers<B: Backend>(
    backend: &mut B,
    host: &str,
    handlers: &[Value],
) -> Result<()> {
    let visible_host = if backend.conservative_sensitive() {
        "[censored]"
    } else {
        host
    };
    for handler in handlers {
        let handler = if backend.conservative_sensitive() {
            json!({"censored":true})
        } else {
            visible(handler)
        };
        backend
            .event(&json!({"host":visible_host,"phase":"handler","planned":true,"task":handler}))?;
    }
    Ok(())
}
