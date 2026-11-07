use super::fact_control::Precise;
use super::*;

#[test]
fn shadowed_parent_lookup_does_not_taint_public_until() -> Result<()> {
    let mut backend = Fake::default();
    let options = Options::default();
    let runner = Runner {
        backend: &mut backend,
        options: &options,
        host: "h".into(),
        scope: TrackedValue::public(json!({"vars":{}})),
        handlers: vec![],
        notified: BTreeMap::new(),
        handling: false,
        recap: Recap::default(),
        next_id: 1,
    };
    let layer = ContextLayer {
        definitions: vec![
            json!({"gate":{"kind":"lookup","provider":"synthetic","key":"fixture"}})
                .as_object()
                .context("parent")?
                .clone(),
            json!({"gate":true}).as_object().context("child")?.clone(),
        ],
        ..ContextLayer::default()
    };
    let condition = json!({"kind":"ref","path":["vars","gate"]});
    assert!(!runner.sensitive_layered_condition(&condition, &runner.scope, &layer));
    let private = json!({"kind":"lookup","provider":"synthetic","key":"fixture"});
    for expression in [
        json!({"kind":"call","function":"if","args":[{"kind":"literal","value":true},{"kind":"literal","value":true},private]}),
        json!({"kind":"call","function":"and","args":[{"kind":"literal","value":false},private]}),
        json!({"kind":"call","function":"or","args":[{"kind":"literal","value":true},private]}),
    ] {
        assert!(!runner.sensitive_layered_condition(&expression, &runner.scope, &layer));
    }
    let reached =
        json!({"kind":"call","function":"and","args":[{"kind":"literal","value":true},private]});
    assert!(runner.sensitive_layered_condition(&reached, &runner.scope, &layer));
    Ok(())
}

#[tokio::test]
async fn lazy_sensitive_until_binding_prevents_agent_cache() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"private gate","noLog":true,"operation":{"kind":"set_fact","values":{"privateGate":true}}},
        {"name":"retry","vars":{"gate":{"kind":"ref","path":["vars","privateGate"]}},"until":{"kind":"ref","path":["vars","gate"]},"operation":{"kind":"command","cmd":"fixture"}}
    ]}]});
    let mut backend = Precise::default();
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert!(backend.0.calls[0].task.no_log);
    assert_eq!(backend.0.events[1]["result"]["censored"], true);
    Ok(())
}

#[tokio::test]
async fn secret_retry_condition_censors_output_and_prevents_agent_cache() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[{"name":"private fact","noLog":true,"operation":{"kind":"set_fact","values":{"gate":{"kind":"literal","value":true}}}},{"name":"retry","until":{"kind":"ref","path":["vars","gate"]},"operation":{"kind":"command","cmd":"public"}}]}]});
    let mut backend = Fake {
        results: [json!({"changed":false,"stdout":"fixture-private-result"})].into(),
        ..Fake::default()
    };
    run(
        &config,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert!(backend.calls[0].task.no_log);
    assert_eq!(backend.events[1]["result"]["censored"], true);
    assert!(!serde_json::to_string(&backend.events)?.contains("fixture-private-result"));
    Ok(())
}

#[test]
fn nested_until_lookup_is_sensitive_before_dispatch() {
    let scope = TrackedValue::object(BTreeMap::from([(
        "vars".into(),
        TrackedValue::object(BTreeMap::from([
            ("private".into(), TrackedValue::secret(json!("fixture"))),
            ("public".into(), TrackedValue::public(json!(true))),
        ])),
    )]));
    assert!(super::super::operation::sensitive_condition(
        &json!({"kind":"call","args":[{"kind":"lookup","provider":"synthetic","key":"fixture"}]}),
        &scope
    ));
    assert!(super::super::operation::sensitive_condition(
        &json!({"kind":"ref","path":["variables","private"]}),
        &scope
    ));
    assert!(!super::super::operation::sensitive_condition(
        &json!({"kind":"ref","path":["vars","public"]}),
        &scope
    ));
}

#[test]
fn known_public_branch_prunes_without_crossing_secret_or_shadowed_bindings() {
    let mut backend = Fake::default();
    let options = Options::default();
    let runner = Runner {
        backend: &mut backend,
        options: &options,
        host: "h".into(),
        scope: TrackedValue::public(json!({})),
        handlers: vec![],
        notified: BTreeMap::new(),
        handling: false,
        recap: Recap::default(),
        next_id: 1,
    };
    let expression = json!({"kind":"call","function":"and","args":[
        {"kind":"ref","path":["vars","gate"]},
        {"kind":"lookup","provider":"synthetic","key":"fixture"}
    ]});
    let make_scope = |gate| {
        TrackedValue::object(BTreeMap::from([(
            "vars".into(),
            TrackedValue::object(BTreeMap::from([("gate".into(), gate)])),
        )]))
    };
    let public = make_scope(TrackedValue::public(json!(false)));
    assert!(!runner.sensitive_layered_condition(&expression, &public, &ContextLayer::default()));
    let secret = make_scope(TrackedValue::secret(json!(false)));
    assert!(runner.sensitive_layered_condition(&expression, &secret, &ContextLayer::default()));
    let shadow = ContextLayer {
        definitions: vec![Map::from_iter([("gate".into(), json!(true))])],
        ..ContextLayer::default()
    };
    assert!(runner.sensitive_layered_condition(&expression, &public, &shadow));
}
