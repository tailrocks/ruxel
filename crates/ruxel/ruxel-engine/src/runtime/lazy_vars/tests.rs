use super::*;
use crate::{
    controller::{Backend, Mode},
    runtime::{Lookups, Output, OutputFormat, RuntimeOptions},
    secrets::Resolver,
    transport,
};
use anyhow::Context;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

fn execution() -> Result<Execution> {
    Ok(Execution {
        connection_failures: 0,
        conservative_sensitive: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        effect_gate: Arc::new(tokio::sync::RwLock::new(())),
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
        template_root: Arc::new(crate::runtime::templates::SourceRoot::open(
            std::path::Path::new("."),
        )?),
        copy_root: Arc::new(crate::runtime::copy_sources::SourceRoot::open(
            std::path::Path::new("."),
        )?),
        output: Arc::new(Output::new(OutputFormat::Json, None)?),
        lookups: Arc::new(Lookups::new(Resolver::synthetic())),
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
        let (_, evaluated) =
            evaluate_layers(&execution, &expression, &scope, std::slice::from_ref(vars))?;
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
    let (value, evaluated) = evaluate_layers(
        &execution,
        &expression,
        &scope,
        std::slice::from_ref(vars.as_object().expect("object")),
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

#[test]
fn layered_bindings_resolve_self_overrides_lazily_with_provenance() -> Result<()> {
    let execution = execution()?;
    let parent = json!({"name":{"kind":"literal","value":"parent"},"unused":{"kind":"ref","path":["results","future"]},"secret":{"kind":"lookup","provider":"synthetic","key":"synthetic-marker"}});
    let child = json!({"name":{"kind":"call","function":"concat","args":[{"kind":"ref","path":["vars","name"]},{"kind":"literal","value":"-child"}]}});
    let layers = [
        parent.as_object().context("parent")?.clone(),
        child.as_object().context("child")?.clone(),
    ];
    let scope = TrackedValue::public(json!({"vars":{},"results":{}}));
    let expression = json!({"kind":"list","items":[{"kind":"ref","path":["vars","name"]},{"kind":"ref","path":["vars","secret"]},{"kind":"ref","path":["vars","secret"]}]});
    let (value, resolved) = evaluate_layers(&execution, &expression, &scope, &layers)?;
    assert_eq!(
        value.redacted(),
        json!(["parent-child", "[REDACTED]", "[REDACTED]"])
    );
    assert_eq!(resolved["name"].value, "parent-child");
    assert!(!resolved.contains_key("unused"));
    assert_eq!(execution.lookups.values.lock().expect("lock").len(), 1);
    Ok(())
}

#[test]
fn layered_binding_cycles_fail_without_resolving_unrelated_definitions() -> Result<()> {
    let execution = execution()?;
    let definitions = json!({"left":{"kind":"ref","path":["vars","right"]},"right":{"kind":"ref","path":["vars","left"]},"safe":{"kind":"literal","value":"ok"}});
    let layers = [definitions.as_object().context("definitions")?.clone()];
    let scope = TrackedValue::public(json!({"vars":{}}));
    let (safe, _) = evaluate_layers(
        &execution,
        &json!({"kind":"ref","path":["vars","safe"]}),
        &scope,
        &layers,
    )?;
    assert_eq!(safe.value, "ok");
    assert!(
        evaluate_layers(
            &execution,
            &json!({"kind":"ref","path":["vars","left"]}),
            &scope,
            &layers
        )
        .is_err()
    );
    Ok(())
}

mod scope_contract;
