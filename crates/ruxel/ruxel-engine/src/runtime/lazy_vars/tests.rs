use super::*;
use crate::{
    controller::Mode,
    runtime::{Lookups, Output, OutputFormat, RuntimeOptions},
    secrets::Resolver,
    transport,
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

fn execution() -> Result<Execution> {
    Ok(Execution {
        inventory: Map::new(),
        runtime: RuntimeOptions {
            agent: PathBuf::new(),
            jobs: 1,
            no_cache: true,
            connection: transport::ConnectOptions::default(),
            output: OutputFormat::Json,
            log_file: None,
            source_dir: PathBuf::new(),
            synthetic_secrets: true,
            local_state: None,
            local_agent: None,
            controller_state: None,
        },
        mode: Mode::Apply,
        connections: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        resources: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        source_gate: Arc::new(tokio::sync::RwLock::new(())),
        output: Arc::new(Output::new(OutputFormat::Json, None)?),
        lookups: Arc::new(Lookups {
            resolver: Resolver::synthetic(),
            values: Mutex::new(Vec::new()),
        }),
    })
}

#[test]
fn lazy_branches_leave_bad_variables_unresolved() -> Result<()> {
    let execution = execution()?;
    let vars = json!({"bad":{"kind":"ref","path":["results","missing"]}});
    let vars = vars.as_object().expect("object");
    let scope = TrackedValue::public(json!({"vars":{}, "results":{}}));
    let bad = json!({"kind":"ref","path":["vars","bad"]});
    for expression in [
        json!({"kind":"call","function":"and","args":[{"kind":"literal","value":false},bad]}),
        json!({"kind":"call","function":"or","args":[{"kind":"literal","value":true},bad]}),
        json!({"kind":"call","function":"if","args":[{"kind":"literal","value":true},{"kind":"list","items":[]},bad]}),
    ] {
        let (_, evaluated) = evaluate(&execution, &expression, &scope, vars)?;
        assert!(evaluated.is_empty());
    }
    Ok(())
}

#[test]
fn inherited_override_and_secret_memo_keep_provenance() -> Result<()> {
    let execution = execution()?;
    let vars = json!({"name":{"kind":"call","function":"concat","args":[{"kind":"ref","path":["vars","name"]},{"kind":"literal","value":"-child"}]},"secret":{"kind":"lookup","provider":"synthetic","key":"synthetic-marker"}});
    let scope = TrackedValue::public(json!({"vars":{"name":"parent"}}));
    let expression = json!({"kind":"list","items":[{"kind":"ref","path":["vars","name"]},{"kind":"ref","path":["vars","secret"]},{"kind":"ref","path":["vars","secret"]}]});
    let (value, evaluated) = evaluate(
        &execution,
        &expression,
        &scope,
        vars.as_object().expect("object"),
    )?;
    assert_eq!(value.value[0], "parent-child");
    assert_eq!(
        value.redacted(),
        json!(["parent-child", "[REDACTED]", "[REDACTED]"])
    );
    assert_eq!(evaluated.len(), 2);
    assert_eq!(execution.lookups.values.lock().expect("lock").len(), 1);
    Ok(())
}
