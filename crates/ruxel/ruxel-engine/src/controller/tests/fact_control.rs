use super::*;

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
