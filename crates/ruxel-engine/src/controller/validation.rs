use super::{Result, Value, array, flag};

pub(super) fn validate(config: &Value) -> Result<()> {
    crate::model::notifications::validate(config)?;
    for play in array(config, "plays") {
        for phase in ["preTasks", "tasks", "handlers"] {
            validate_tasks(array(play, phase))?;
        }
    }
    Ok(())
}

fn validate_tasks(tasks: &[Value]) -> Result<()> {
    for task in tasks {
        crate::model::validate_block_result_controls(
            !array(task, "block").is_empty(),
            task.get("register").is_some_and(|value| !value.is_null()),
            flag(task, "ignoreErrors"),
        )?;
        for section in ["block", "rescue", "always"] {
            validate_tasks(array(task, section))?;
        }
    }
    Ok(())
}
