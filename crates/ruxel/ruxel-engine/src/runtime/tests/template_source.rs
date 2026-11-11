use super::*;
use std::path::PathBuf;

mod confinement;

fn template_execution(directory: &std::path::Path) -> Result<Execution> {
    Ok(Execution {
        connection_failures: 0,
        conservative_sensitive: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        effect_gate: Arc::new(tokio::sync::RwLock::new(())),
        output: Arc::new(Output::new(OutputFormat::Json, None)?),
        inventory: Map::new(),
        mode: Mode::Apply,
        runtime: RuntimeOptions {
            agent: PathBuf::from("unused"),
            source_dir: directory.to_path_buf(),
            jobs: 1,
            no_cache: false,
            connection: transport::ConnectOptions::default(),
            output: OutputFormat::Json,
            log_file: None,
            synthetic_secrets: true,
            local_state: None,
            local_agent: None,
            controller_state: None,
        },
        connections: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        resources: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        source_gate: Arc::new(tokio::sync::RwLock::new(())),
        template_root: Arc::new(templates::SourceRoot::open(directory)?),
        copy_root: Arc::new(copy_sources::SourceRoot::open(directory)?),
        lookups: Arc::new(Lookups::new(Resolver::synthetic())),
    })
}

fn check_template_rendering(execution: &mut Execution, directory: &std::path::Path) -> Result<()> {
    let tracked = execution.evaluate_tracked(
        &json!({"public":"visible", "private":{"kind":"call", "function":"base64Encode", "args":[{"kind":"lookup", "provider":"synthetic", "key":"fixture-only"}]}}),
        &TrackedValue::public(json!({})),
    )?;
    assert_eq!(
        tracked.redacted(),
        json!({"public":"visible", "private":"[REDACTED]"})
    );
    assert_ne!(tracked.value["private"], "[REDACTED]");
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
    for text in ["rendered", "rendered\n", "rendered\n\n"] {
        let scope = json!({"vars":{"name":text}})
            .as_object()
            .context("scope")?
            .clone();
        let mut params = json!({"src":"source.pkl","dest":"unused"})
            .as_object()
            .context("params")?
            .clone();
        execution.prepare_operation("template", &mut params, &scope)?;
        let value = execution.evaluate(&Value::Object(params), &scope)?;
        assert_eq!(value["content"], text);
    }
    let mut jinja = json!({"src":"source.j2","dest":"unused"})
        .as_object()
        .context("params")?
        .clone();
    assert!(
        execution
            .prepare_operation("template", &mut jinja, &scope)
            .is_err()
    );
    let dynamic_scope = json!({"vars":{"source":"source.pkl", "name":"dynamic"}})
        .as_object()
        .context("scope")?
        .clone();
    let mut dynamic = json!({"src":{"kind":"ref","path":["vars","source"]}})
        .as_object()
        .context("params")?
        .clone();
    execution.prepare_operation("template", &mut dynamic, &dynamic_scope)?;
    assert_eq!(
        execution.evaluate(&Value::Object(dynamic), &dynamic_scope)?["content"],
        "dynamic"
    );
    let mut absolute = json!({"src":directory.join("source.pkl")})
        .as_object()
        .context("params")?
        .clone();
    execution.prepare_operation("template", &mut absolute, &scope)?;
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
    let mut execution = template_execution(&directory)?;
    check_template_rendering(&mut execution, &directory)?;
    confinement::check_template_confinement(&mut execution, &directory)?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
