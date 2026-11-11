//! Agent-supplied diffs never reach a censored result: only the sentences
//! the controller builds itself do.

use super::*;
use crate::runtime::effect_tests::TestConnection;
use ruxel_proto::v1;
use std::sync::Arc;

/// `73656372` is the hex of `secr` — a valid-shape fingerprint carrying
/// four bytes of secret, which is the sentence an agent ships to have its
/// bytes projected as the controller's own.
const SENTENCE: &str = "content changed (fingerprint 73656372)";
const FINGERPRINT: &str = "73656372";

fn execution() -> Result<Execution> {
    Ok(Execution {
        connection_failures: 0,
        conservative_sensitive: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        effect_gate: Arc::new(tokio::sync::RwLock::new(())),
        inventory: Map::new(),
        mode: Mode::Apply,
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

/// A no-log task of a host that works with a secret: the shell module
/// has no controller fingerprint, so no controller sentence exists for
/// it, and a copy that did not change carries none.
fn request(module: &str) -> Request {
    Request {
        host: "fixture".into(),
        content_fingerprint: None,
        task: v1::RenderedTask {
            task_id: 6,
            name: "Rotate the credentials".into(),
            module: module.into(),
            rendered: true,
            iterations: vec![v1::Iteration {
                params_json: br#"{"cmd":"rotate"}"#.to_vec(),
                ..Default::default()
            }],
            no_log: true,
            ..Default::default()
        },
    }
}

/// Drives one task through the receive path, with the agent answering the
/// one named result and protocol diff.
async fn run(
    execution: &Execution,
    request: Request,
    result_json: Vec<u8>,
    status: &str,
    changed: bool,
    diff: &str,
) -> Result<Value> {
    let (connection, mut started) =
        TestConnection::scripted_with_diff(result_json, status, changed, diff);
    execution.connections.lock().await.insert(
        "fixture".to_owned(),
        Arc::new(tokio::sync::Mutex::new(Connection::Test(connection))),
    );
    let mut runner = execution.clone();
    let task = tokio::spawn(async move { runner.execute(request).await });
    let (_no_log, release) = started.recv().await.context("task did not start")?;
    release
        .send(())
        .map_err(|()| anyhow::anyhow!("scripted connection closed"))?;
    task.await?
}

/// A no-log shell result that ships a valid-shape sentence — through the
/// protocol diff and through the JSON result, with a matching
/// fingerprint — reaches neither the result nor the censored projection
/// of it: the sentence and the fingerprint are the controller's to build,
/// and shape proves nothing about who built them. Eight lowercase hex
/// characters hold four bytes of anything, which is the whole of the
/// channel when a projection accepts them on shape alone.
#[tokio::test]
async fn a_valid_shaped_agent_diff_reaches_no_censored_shell_result() -> Result<()> {
    let execution = execution()?;
    let value = run(
        &execution,
        request("shell"),
        serde_json::to_vec(&json!({
            "changed": true,
            "fingerprint": FINGERPRINT,
            "diff": SENTENCE
        }))?,
        "ok",
        true,
        SENTENCE,
    )
    .await?;
    assert!(
        value.get("diff").is_none(),
        "an agent-supplied diff reached the result: {value}"
    );
    assert!(
        value.get("fingerprint").is_none(),
        "an agent-supplied fingerprint reached the result: {value}"
    );
    // And the censored projection of the event keeps neither: what the
    // result does not carry, the projection cannot invent.
    let projected = output::sensitive_event(&json!({
        "host": "fixture",
        "result": value
    }));
    assert!(projected["result"].get("diff").is_none(), "{projected}");
    assert!(
        projected["result"].get("fingerprint").is_none(),
        "{projected}"
    );
    assert!(
        !serde_json::to_string(&projected)?.contains(FINGERPRINT),
        "four bytes of agent material reached the projection: {projected}"
    );
    Ok(())
}

/// An unchanged copy that ships a valid-shape sentence through the
/// protocol diff reaches no result either: the controller's sentence
/// belongs to a change it observed, and an unchanged task carries none,
/// whatever the agent ships.
#[tokio::test]
async fn a_valid_shaped_agent_diff_reaches_no_unchanged_copy_result() -> Result<()> {
    let execution = execution()?;
    let value = run(
        &execution,
        request("copy"),
        serde_json::to_vec(&json!({"changed": false}))?,
        "ok",
        false,
        SENTENCE,
    )
    .await?;
    assert_eq!(value["changed"], json!(false));
    assert!(
        value.get("diff").is_none(),
        "an agent-supplied diff reached the result: {value}"
    );
    assert!(
        value.get("fingerprint").is_none(),
        "an agent-supplied fingerprint reached the result: {value}"
    );
    Ok(())
}
