use super::*;
use crate::runtime::effect_tests::TestConnection;
use ruxel_proto::v1;
use std::sync::Arc;

/// A secret-bearing file that changed shows the fingerprint sentence: the
/// agent ships no diff for a no-log task, so the controller builds the
/// sentence from the fingerprint it read from the selectors before the
/// content was evaluated.
#[tokio::test]
async fn a_changed_secret_bearing_copy_shows_the_fingerprint() -> Result<()> {
    let execution = execution()?;
    let (connection, mut started) =
        TestConnection::scripted_result(br#"{"changed":true}"#.to_vec(), "ok", true);
    execution.connections.lock().await.insert(
        "fixture".to_owned(),
        Arc::new(tokio::sync::Mutex::new(Connection::Test(connection))),
    );
    let mut runner = execution.clone();
    let task = tokio::spawn(async move { runner.execute(request()).await });
    let (_no_log, release) = started.recv().await.context("copy task did not start")?;
    release
        .send(())
        .map_err(|()| anyhow::anyhow!("scripted connection closed"))?;
    let value = task.await??;
    assert_eq!(
        value["diff"],
        json!("content changed (fingerprint abcd1234)")
    );
    assert_eq!(value["fingerprint"], json!("abcd1234"));
    assert_eq!(value["changed"], json!(true));
    Ok(())
}

/// A copy that did not change shows no fingerprint: the sentence belongs to
/// a change.
#[tokio::test]
async fn an_unchanged_secret_bearing_copy_shows_no_fingerprint() -> Result<()> {
    let execution = execution()?;
    let (connection, mut started) =
        TestConnection::scripted_result(br#"{"changed":false}"#.to_vec(), "ok", false);
    execution.connections.lock().await.insert(
        "fixture".to_owned(),
        Arc::new(tokio::sync::Mutex::new(Connection::Test(connection))),
    );
    let mut runner = execution.clone();
    let task = tokio::spawn(async move { runner.execute(request()).await });
    let (_no_log, release) = started.recv().await.context("copy task did not start")?;
    release
        .send(())
        .map_err(|()| anyhow::anyhow!("scripted connection closed"))?;
    let value = task.await??;
    assert!(value.get("diff").is_none(), "no diff: {value}");
    assert!(
        value.get("fingerprint").is_none(),
        "no fingerprint: {value}"
    );
    Ok(())
}

fn execution() -> Result<Execution> {
    Ok(Execution {
        connection_failures: 0,
        conservative_sensitive: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        effect_gate: Arc::new(tokio::sync::RwLock::new(())),
        inventory: Map::new(),
        mode: Mode::Check,
        runtime: RuntimeOptions {
            agent: "unused".into(),
            source_dir: ".".into(),
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
        template_root: Arc::new(templates::SourceRoot::open(std::path::Path::new("."))?),
        copy_root: Arc::new(copy_sources::SourceRoot::open(std::path::Path::new("."))?),
        output: Arc::new(Output::new(OutputFormat::Json, None)?),
        lookups: Arc::new(Lookups::new(
            Resolver::synthetic().expect("synthetic resolver"),
        )),
    })
}

fn request() -> Request {
    Request {
        host: "fixture".into(),
        content_fingerprint: Some("abcd1234".to_owned()),
        task: v1::RenderedTask {
            task_id: 4,
            name: "Write the env file".into(),
            module: "copy".into(),
            rendered: true,
            iterations: vec![v1::Iteration {
                params_json: br#"{"dest":"/etc/app/env","content":"dry-secret-0123"}"#.to_vec(),
                ..Default::default()
            }],
            no_log: true,
            ..Default::default()
        },
    }
}
