use super::*;
use std::path::PathBuf;

mod prefetch;

#[test]
fn sensitive_event_exposes_only_safe_metadata() -> Result<()> {
    let projected = output::sensitive_event(
        &json!({"host":"synthetic-marker", "task":"synthetic-marker", "item":"synthetic-marker", "pause":"synthetic-marker", "result":{"msg":"synthetic-marker", "changed":true, "failed":false}}),
    );
    assert!(!serde_json::to_string(&projected)?.contains("synthetic-marker"));
    assert_eq!(projected["host"], "[censored]");
    assert_eq!(projected["task"], "[censored]");
    assert_eq!(projected["result"]["changed"], true);
    assert_eq!(projected["result"]["failed"], false);
    Ok(())
}

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
        lookups: Arc::new(Lookups {
            resolver: Resolver::synthetic(),
            values: Mutex::new(Vec::new()),
        }),
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

fn check_template_confinement(
    execution: &mut Execution,
    directory: &std::path::Path,
) -> Result<()> {
    let scope = json!({"vars":{"name":"rendered-value"}})
        .as_object()
        .context("scope")?
        .clone();
    #[cfg(unix)]
    {
        let outside = directory.with_extension("outside.pkl");
        std::fs::write(&outside, "output { value = \"outside\" }")?;
        let traversal = format!(
            "../{}",
            outside
                .file_name()
                .context("outside name")?
                .to_string_lossy()
        );
        std::os::unix::fs::symlink("source.pkl", directory.join("link.pkl"))?;
        std::os::unix::fs::symlink(directory, directory.join("linkdir"))?;
        for source in [
            traversal.as_str(),
            outside.to_str().context("outside path")?,
            "link.pkl",
            "linkdir/source.pkl",
        ] {
            let mut params = json!({"src": source})
                .as_object()
                .context("params")?
                .clone();
            assert!(
                execution
                    .prepare_operation("template", &mut params, &scope)
                    .is_err(),
                "accepted {source}"
            );
        }
        std::fs::remove_file(outside)?;
    }
    let moved = directory.with_extension("moved");
    std::fs::rename(directory, &moved)?;
    std::fs::create_dir(directory)?;
    std::fs::write(
        directory.join("source.pkl"),
        "output { value = \"replacement\" }",
    )?;
    let mut pinned = json!({"src":"source.pkl"})
        .as_object()
        .context("params")?
        .clone();
    execution.prepare_operation("template", &mut pinned, &scope)?;
    assert_eq!(
        execution.evaluate(&Value::Object(pinned), &scope)?["content"],
        "rendered-value"
    );
    std::fs::remove_dir_all(moved)?;
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
    check_template_confinement(&mut execution, &directory)?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn private_event_log_records_redacted_structure_and_refuses_overwrite() -> Result<()> {
    let path = std::env::temp_dir().join(format!("ruxel-events-{}.jsonl", std::process::id()));
    let lookups = Lookups {
        resolver: Resolver::synthetic(),
        values: Mutex::new(vec!["synthetic-private-value".into()]),
    };
    let output = Output::new(OutputFormat::Human, Some(&path))?;
    output.event(&lookups.redact(
        &json!({"host":"fixture","result":{"value":"synthetic-private-value","failed":false}}),
    )?)?;
    let text = std::fs::read_to_string(&path)?;
    let value: Value = serde_json::from_str(&text)?;
    assert_eq!(value["result"]["value"], "[REDACTED]");
    assert_eq!(value["result"]["failed"], false);
    assert!(Output::new(OutputFormat::Json, Some(&path)).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777,
            0o600
        );
    }
    drop(output);
    std::fs::remove_file(path)?;
    Ok(())
}
