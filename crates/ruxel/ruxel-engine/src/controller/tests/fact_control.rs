use super::*;

#[derive(Default)]
pub(super) struct Precise(pub(super) Fake);
impl Backend for Precise {
    fn evaluate(&mut self, value: &Value, scope: &Map<String, Value>) -> Result<Value> {
        self.0.evaluate(value, scope)
    }
    fn evaluate_tracked(&mut self, value: &Value, scope: &TrackedValue) -> Result<TrackedValue> {
        if value.get("kind").and_then(Value::as_str) == Some("ref") {
            let mut current = scope.clone();
            for part in value["path"].as_array().context("path")? {
                current = current
                    .get(part.as_str().context("key")?)
                    .context("missing")?;
            }
            Ok(current)
        } else {
            Ok(TrackedValue::public(self.evaluate(
                value,
                scope.value.as_object().context("scope")?,
            )?))
        }
    }
    async fn execute(&mut self, request: Request) -> Result<Value> {
        self.0.execute(request).await
    }
    fn event(&mut self, event: &Value) -> Result<()> {
        self.0.event(event)
    }
}

#[tokio::test]
async fn sensitive_control_taints_stored_fact_without_tainting_public_siblings() -> Result<()> {
    let mut backend = Fake::default();
    let options = Options::default();
    let mut runner = Runner {
        backend: &mut backend,
        options: &options,
        host: "h".into(),
        scope: TrackedValue::public(json!({})),
        handlers: vec![],
        notified: BTreeSet::new(),
        handling: false,
        recap: Recap::default(),
        next_id: 1,
    };
    for (name, private) in [("public", false), ("guarded", true)] {
        let layer = ContextLayer {
            control_sensitive: private,
            ..ContextLayer::default()
        };
        let params = json!({"values":{(name):"synthetic"}});
        let tracked = TrackedValue::public(params.clone());
        runner
            .local_once(
                &json!({}),
                "set_fact",
                &layer,
                params.as_object().context("params")?,
                &tracked,
                private,
            )
            .await?;
    }
    let facts = runner.scope.get("setFacts").context("facts")?;
    assert!(!facts.get("public").context("public")?.is_secret());
    assert!(facts.get("guarded").context("guarded")?.is_secret());
    Ok(())
}

#[tokio::test]
async fn secret_loop_item_taints_constant_fact_without_tainting_public_sibling() -> Result<()> {
    let config = json!({"inventory":{"hosts":{"h":{}},"groups":{}},"plays":[{"targets":["h"],"tasks":[
        {"name":"loop","loop":{"kind":"ref","path":["vars","items"]},"operation":{"kind":"set_fact","values":{"chosen":"synthetic"}}}
    ]}]});
    for private in [false, true] {
        let mut backend = Precise::default();
        let mut facts = BTreeMap::from([(
            "h".into(),
            TrackedValue::object(BTreeMap::from([
                (
                    "items".into(),
                    if private {
                        TrackedValue::secret(json!(["fixture"]))
                    } else {
                        TrackedValue::public(json!(["fixture"]))
                    },
                ),
                ("sibling".into(), TrackedValue::public(json!("public"))),
            ])),
        )]);
        run(
            &config,
            &BTreeMap::new(),
            &Options::default(),
            &mut backend,
            &mut facts,
        )
        .await?;
        let result = &facts["h"];
        assert_eq!(result.get("chosen").context("chosen")?.is_secret(), private);
        assert!(!result.get("sibling").context("sibling")?.is_secret());
    }
    Ok(())
}
