pub(crate) mod notifications;
mod operations;
mod shapes;
use anyhow::{Context, Result, ensure};
use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

mod config;
use config::{Config, Task};

#[derive(Clone, Copy)]
enum FieldType {
    Flags,
    Scalar,
    Text,
    HttpUrl,
    Timezone,
    SysctlKey,
    Flag,
    Number,
    Port,
    Mode,
    TextList,
    PrivilegeList,
    StringList,
    NonEmptyStringList,
    ExprMap,
    Choice(&'static [&'static str]),
}
struct Field {
    name: &'static str,
    required: bool,
    kind: FieldType,
}
struct Operation {
    kind: &'static str,
    fields: &'static [Field],
}

mod expressions;
use expressions::{expression, text_list};

fn validate_field(value: &Value, kind: FieldType, deferred: bool) -> Result<()> {
    if value.is_object() && !matches!(kind, FieldType::ExprMap) {
        ensure!(deferred, "unresolved operation expression");
        return expression(value);
    }
    let valid = match kind {
        FieldType::Scalar => value.is_string() || value.is_boolean() || value.is_number(),
        FieldType::Flags => {
            if value.is_boolean() {
                true
            } else {
                for flag in value
                    .as_array()
                    .context("assert requires boolean or list")?
                {
                    if deferred && flag.is_object() {
                        expression(flag)?;
                    } else {
                        ensure!(flag.is_boolean(), "assert requires boolean values");
                    }
                }
                true
            }
        }
        FieldType::Text => value.is_string(),
        FieldType::HttpUrl => value
            .as_str()
            .is_some_and(|s| s.starts_with("http://") || s.starts_with("https://")),
        FieldType::Timezone => value
            .as_str()
            .is_some_and(|s| !s.is_empty() && !s.starts_with('-')),
        FieldType::SysctlKey => value.as_str().is_some_and(valid_sysctl_key),
        FieldType::Flag => value.is_boolean(),
        FieldType::Number => value.as_u64().is_some(),
        FieldType::Port => value.as_u64().is_some_and(|n| (1..=65535).contains(&n)),
        FieldType::Mode => value.as_str().is_some_and(|s| {
            (3..=4).contains(&s.len()) && s.bytes().all(|b| (b'0'..=b'7').contains(&b))
        }),
        FieldType::TextList => text_list(value),
        FieldType::PrivilegeList => {
            value
                .as_str()
                .is_some_and(|s| s.split(',').any(|part| !part.trim().is_empty()))
                || value.as_array().is_some_and(|items| {
                    !items.is_empty()
                        && items
                            .iter()
                            .all(|item| item.as_str().is_some_and(|s| !s.trim().is_empty()))
                })
        }
        FieldType::StringList => value
            .as_array()
            .is_some_and(|v| v.iter().all(Value::is_string)),
        FieldType::NonEmptyStringList => value
            .as_array()
            .is_some_and(|v| !v.is_empty() && v.iter().all(Value::is_string)),
        FieldType::Choice(choices) => value.as_str().is_some_and(|s| choices.contains(&s)),
        FieldType::ExprMap => {
            for expr in value
                .as_object()
                .context("expected expression map")?
                .values()
            {
                if deferred {
                    expression(expr)?;
                }
            }
            true
        }
    };
    ensure!(valid, "invalid operation parameter type or value");
    Ok(())
}
fn operation(kind: &str, params: &Map<String, Value>, deferred: bool) -> Result<()> {
    let spec = operations::OPERATIONS
        .iter()
        .find(|s| s.kind == kind)
        .context("unknown operation kind")?;
    ensure!(
        params
            .keys()
            .all(|key| spec.fields.iter().any(|f| f.name == key)),
        "unknown operation parameter"
    );
    for field in spec.fields {
        match params.get(field.name).filter(|v| !v.is_null()) {
            Some(value) => validate_field(value, field.kind, deferred)?,
            None => ensure!(!field.required, "missing required operation parameter"),
        }
    }
    shapes::validate(kind, params, deferred)?;
    Ok(())
}

/// # Errors
/// Rejects unknown operations, fields, invalid values and unresolved expressions.
pub fn validate_operation(kind: &str, params: &Map<String, Value>) -> Result<()> {
    operation(kind, params, false)
}

fn expression_map(values: &BTreeMap<String, Value>) -> Result<()> {
    for value in values.values() {
        expression(value)?;
    }
    Ok(())
}
fn environment(values: &BTreeMap<String, Value>) -> Result<()> {
    ensure!(
        values
            .keys()
            .all(|key| !key.is_empty() && !key.contains(['=', '\0'])),
        "invalid environment variable name"
    );
    expression_map(values)
}
fn validate_become_user(user: Option<&str>) -> Result<()> {
    ensure!(
        user.is_none_or(|user| !user.is_empty()
            && !user.starts_with('-')
            && user
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))),
        "invalid become user"
    );
    Ok(())
}
pub(crate) fn validate_block_result_controls(
    block: bool,
    registered: bool,
    ignore_errors: bool,
) -> Result<()> {
    ensure!(!block || !registered, "register requires an operation task");
    ensure!(
        !block || !ignore_errors,
        "ignoreErrors requires an operation task"
    );
    Ok(())
}
fn valid_sysctl_key(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:-".contains(&b))
}
fn validate_delegation(task: &Task, hosts: &IndexMap<String, config::Host>) -> Result<()> {
    if let Some(destination) = &task.delegate_to {
        ensure!(
            task.delegate_allow.as_ref().is_some_and(
                |allowed| !allowed.is_empty() && allowed.iter().all(|host| !host.is_empty())
            ),
            "delegation requires an explicit nonempty host allowlist"
        );
        if destination.is_object() {
            expression(destination)?;
        } else {
            ensure!(
                destination.is_string(),
                "delegation target must be text or expression"
            );
            let host = destination
                .as_str()
                .context("delegation target must be text")?;
            ensure!(
                host == "localhost" || hosts.contains_key(host),
                "literal delegation target is not in inventory"
            );
            ensure!(
                task.delegate_allow
                    .as_ref()
                    .is_some_and(|allowed| allowed.iter().any(|allowed| allowed == host)),
                "delegation target is not explicitly authorized"
            );
            ensure!(
                host != "localhost" || task.delegate_localhost == Some(true),
                "localhost delegation requires explicit opt-in"
            );
        }
    } else {
        ensure!(
            task.delegate_allow.is_none() && task.delegate_localhost.is_none(),
            "delegation authorization requires delegateTo on the same task"
        );
    }
    Ok(())
}
fn validate_tasks(tasks: &[Task], hosts: &IndexMap<String, config::Host>) -> Result<()> {
    for task in tasks {
        validate_become_user(task.become_user.as_deref())?;
        validate_block_result_controls(
            !task.block.is_empty(),
            task.register.is_some(),
            task.ignore_errors,
        )?;
        ensure!(
            task.operation.is_some() == task.block.is_empty(),
            "task requires operation or nonempty block"
        );
        ensure!(
            task.operation.is_none() || (task.rescue.is_empty() && task.always.is_empty()),
            "rescue and always require a block task"
        );
        ensure!(
            !matches!(
                task.loop_var.as_str(),
                "vars" | "facts" | "results" | "inventoryHost" | "setFacts"
            ),
            "loop variable shadows a reserved scope root"
        );
        ensure!(
            task.block.is_empty()
                || (task.loop_expr.is_none()
                    && task.loop_label.is_none()
                    && task.loop_var == "item"),
            "loop controls require an operation task"
        );
        ensure!(
            task.block.is_empty()
                || (task.until.is_none()
                    && task.retries.is_none()
                    && task.changed_when.is_none()
                    && task.failed_when.is_none()),
            "retry and result controls require an operation task"
        );
        ensure!(
            task.retries != Some(u64::MAX),
            "retries exceed supported range"
        );
        validate_delegation(task, hosts)?;
        for expr in [
            &task.when,
            &task.loop_expr,
            &task.loop_label,
            &task.until,
            &task.changed_when,
            &task.failed_when,
        ]
        .into_iter()
        .flatten()
        {
            expression(expr)?;
        }
        expression_map(&task.vars)?;
        environment(&task.environment)?;
        if let Some(value) = &task.operation {
            let mut params = value
                .as_object()
                .context("operation must be an object")?
                .clone();
            let kind = params
                .remove("kind")
                .and_then(|v| v.as_str().map(str::to_owned))
                .context("operation requires kind")?;
            operation(&kind, &params, true)?;
        }
        validate_tasks(&task.block, hosts)?;
        validate_tasks(&task.rescue, hosts)?;
        validate_tasks(&task.always, hosts)?;
    }
    Ok(())
}
/// # Errors
/// Rejects malformed configuration, unknown fields and invalid typed variants.
pub fn validate_config(value: &Value) -> Result<Value> {
    let value = if let Some(config) = value.get("config") {
        ensure!(
            value.as_object().is_some_and(|o| o.len() == 1),
            "unknown root output field"
        );
        config
    } else {
        value
    };
    let config: Config =
        serde_json::from_value(value.clone()).context("invalid configuration shape")?;
    for host in config.inventory.hosts.values() {
        ensure!(
            !host.address.is_empty()
                && !host.address.starts_with('-')
                && host.port != 0
                && host
                    .address
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-:%[]".contains(&b)),
            "invalid host address or port"
        );
        ensure!(
            host.user.as_ref().is_none_or(|user| !user.is_empty()
                && !user.starts_with('-')
                && user
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))),
            "invalid SSH user"
        );
        expression_map(&host.vars)?;
    }
    for group in config.inventory.groups.values() {
        expression_map(&group.vars)?;
    }
    for play in &config.plays {
        validate_become_user(play.become_user.as_deref())?;
        expression_map(&play.vars)?;
        environment(&play.environment)?;
        validate_tasks(&play.pre_tasks, &config.inventory.hosts)?;
        validate_tasks(&play.tasks, &config.inventory.hosts)?;
        validate_tasks(&play.handlers, &config.inventory.hosts)?;
    }
    let normalized = serde_json::to_value(config).context("normalize configuration")?;
    notifications::validate(&normalized)?;
    Ok(normalized)
}

#[cfg(test)]
mod tests;

pub(crate) fn validate_template(value: &Value) -> Result<()> {
    if value.is_string() {
        Ok(())
    } else {
        expression(value)
    }
}
