use super::*;

#[test]
fn redaction_preserves_json_structure_for_short_and_escaped_secrets() -> Result<()> {
    let lookups = Lookups {
        resolver: Resolver::synthetic(),
        values: Mutex::new(vec!["true".into(), "quoted\"secret".into()]),
    };
    let result =
        lookups.redact(&json!({"value":true,"nested":["true","quoted\"secret"],"true-key":1}))?;
    assert_eq!(result["value"], true);
    assert_eq!(result["nested"], json!(["[REDACTED]", "[REDACTED]"]));
    assert_eq!(result["[REDACTED]-key"], 1);
    Ok(())
}

#[test]
fn typed_template_source_resolves_task_scope_and_rejects_jinja() -> Result<()> {
    let directory =
        std::env::temp_dir().join(format!("ruxel-native-template-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("source.pkl"),
        "output { value = new Dynamic { kind = \"ref\"; path = new Listing { \"vars\"; \"name\" } } }",
    )?;
    let mut execution = Execution {
        inventory: Map::new(),
        mode: Mode::Apply,
        runtime: RuntimeOptions {
            agent: PathBuf::from("unused"),
            source_dir: directory.clone(),
            jobs: 1,
            synthetic_secrets: true,
            local_state: None,
            local_agent: None,
            controller_state: None,
        },
        connections: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        lookups: Arc::new(Lookups {
            resolver: Resolver::synthetic(),
            values: Mutex::new(Vec::new()),
        }),
    };
    let scope = json!({"vars":{"name":"rendered-value"}})
        .as_object()
        .context("scope")?
        .clone();
    let mut params = json!({"src":"source.pkl","dest":"unused"})
        .as_object()
        .context("params")?
        .clone();
    execution.prepare_operation("template", &mut params, &scope)?;
    let value = execution.evaluate(&Value::Object(params), &scope)?;
    assert_eq!(value["content"], "rendered-value");
    assert!(value.get("src").is_none());
    let mut jinja = json!({"src":"source.j2","dest":"unused"})
        .as_object()
        .context("params")?
        .clone();
    assert!(
        execution
            .prepare_operation("template", &mut jinja, &scope)
            .is_err()
    );
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
