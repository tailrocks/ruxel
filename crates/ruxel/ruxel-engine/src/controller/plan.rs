use super::{Backend, Result, Value, array, flag, json, string};

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
    if !filter.is_empty()
        && !tags
            .iter()
            .any(|tag| tag == "always" || filter.contains(tag))
    {
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
    for task in tasks {
        if let Some((selected, task_count)) = project(task, filter, inherited) {
            backend.event(&json!({"host":host,"phase":phase,"planned":true,"task":selected}))?;
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
    for handler in handlers {
        backend.event(
            &json!({"host":host,"phase":"handler","planned":true,"task":visible(handler)}),
        )?;
    }
    Ok(())
}
