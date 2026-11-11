use super::*;
use crate::runtime::effect_tests::TestConnection;
use ruxel_proto::v1;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fake fnox store: `set` counts its calls and keeps the value, `get`
/// answers the kept value. The fixture writes its own values; no real
/// secret ever reaches the store.
#[derive(Default)]
struct Store {
    sets: AtomicUsize,
    gets: AtomicUsize,
    values: Mutex<std::collections::HashMap<String, String>>,
}
impl crate::secrets::Backend for Store {
    fn fnox_get(&self, key: &str) -> Result<Option<String>, String> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .values
            .lock()
            .map_err(|_| "poisoned store")?
            .get(key)
            .cloned())
    }
    fn fnox_set(&self, key: &str, value: &str) -> Result<(), String> {
        self.sets.fetch_add(1, Ordering::SeqCst);
        self.values
            .lock()
            .map_err(|_| "poisoned store")?
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }
    fn onepassword_fields(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: &str,
    ) -> Result<serde_json::Value, String> {
        Err("unexpected 1Password fetch".into())
    }
    fn onepassword_reference(&self, _: &str) -> Result<String, String> {
        Err("unexpected 1Password reference".into())
    }
    fn command(&self, _: &[String]) -> Result<String, String> {
        Err("unexpected command lookup".into())
    }
}

fn execution(lookups: Lookups, log: Option<&std::path::Path>) -> Result<Execution> {
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
            synthetic_secrets: false,
            local_state: None,
            local_agent: None,
            controller_state: None,
        },
        connections: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        resources: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        source_gate: Arc::new(tokio::sync::RwLock::new(())),
        template_root: Arc::new(templates::SourceRoot::open(std::path::Path::new("."))?),
        copy_root: Arc::new(copy_sources::SourceRoot::open(std::path::Path::new("."))?),
        output: Arc::new(Output::new(OutputFormat::Json, log)?),
        lookups: Arc::new(lookups),
    })
}

fn request() -> Request {
    Request {
        host: "fixture".into(),
        content_fingerprint: None,
        task: v1::RenderedTask {
            task_id: 9,
            name: "Store the cluster admin kubeconfig".into(),
            module: "fetchSecret".into(),
            rendered: true,
            iterations: vec![v1::Iteration {
                params_json:
                    br#"{"path":"/etc/kubernetes/admin.conf","key":"K8S_ADMIN_KUBECONFIG"}"#
                        .to_vec(),
                ..Default::default()
            }],
            no_log: true,
            ..Default::default()
        },
    }
}

/// Drives one `FetchSecret` task through the receive path, with the agent
/// answering the one named slurp result.
async fn run_fetch(execution: &Execution, result_json: Vec<u8>) -> Result<Value> {
    run_fetch_with_status(execution, result_json, "ok").await
}

/// Drives one `FetchSecret` task through the receive path, with the agent
/// answering the one named result and status.
async fn run_fetch_with_status(
    execution: &Execution,
    result_json: Vec<u8>,
    status: &str,
) -> Result<Value> {
    let (connection, mut started) = TestConnection::scripted_with_status(result_json, status);
    execution.connections.lock().await.insert(
        "fixture".to_owned(),
        Arc::new(tokio::sync::Mutex::new(Connection::Test(connection))),
    );
    let mut runner = execution.clone();
    let task = tokio::spawn(async move { runner.execute(request()).await });
    let (_no_log, release) = started.recv().await.context("fetch task did not start")?;
    release
        .send(())
        .map_err(|()| anyhow::anyhow!("scripted connection closed"))?;
    task.await?
}

/// The forms of the fetched value this test greps for: the exact value,
/// each of its lines, and the standard and URL-safe base64 and the hex
/// encoding of the value and of each of its lines — the registry covers
/// these and more (upper-case hex, `urlencode`).
fn derived_forms(plaintext: &str, wire: &str) -> Vec<String> {
    let hex = |text: &str| -> String {
        text.bytes()
            .flat_map(|byte| [byte >> 4, byte & 0x0f])
            .map(|nibble| char::from_digit(u32::from(nibble), 16).expect("hex digit"))
            .collect()
    };
    let mut forms = vec![
        plaintext.to_owned(),
        wire.to_owned(),
        wire.trim_end_matches('=').to_owned(),
        wire.replace('+', "-").replace('/', "_"),
        hex(plaintext),
    ];
    for line in plaintext.lines() {
        let line_wire = STANDARD.encode(line);
        forms.push(line.to_owned());
        forms.push(line_wire.trim_end_matches('=').to_owned());
        forms.push(line_wire.replace('+', "-").replace('/', "_"));
        forms.push(hex(line));
    }
    forms
}

/// The fetched plaintext and its base64 reach no result, no stdout and no
/// event log in any derived form: the content never leaves
/// `receive_fetch_secret`, and the fetched value joins the redaction
/// registry, so a later task that echoes one is still censored. The grep
/// runs over the captured results and output — the `FetchSecret` twin of
/// the lookup no-plaintext test.
#[tokio::test]
async fn fetched_plaintext_reaches_no_result_output_or_log_in_any_derived_form() -> Result<()> {
    const PLAINTEXT: &str = "apiVersion: v1\nkind: Config\nkubeconfig-fixture-9c21\n";
    let wire = STANDARD.encode(PLAINTEXT);
    let store = Arc::new(Store::default());
    let log = std::env::temp_dir().join(format!(
        "ruxel-fetch-no-plaintext-{}.jsonl",
        std::process::id()
    ));
    drop(std::fs::remove_file(&log));
    let execution = execution(
        Lookups::new(Resolver::new(store.clone()).expect("resolver")),
        Some(&log),
    )?;
    let value = run_fetch(&execution, serde_json::to_vec(&json!({"content": wire}))?).await?;
    assert!(
        value.get("content").is_none(),
        "the result carries the fetched content: {value}"
    );
    assert_eq!(value["changed"], json!(true), "apply stores the fetch");
    assert_eq!(store.sets.load(Ordering::SeqCst), 1, "one store write");
    let mut captured = serde_json::to_string(&value)?;
    // One line of the fetched document, echoed on its own: a later task
    // that prints a single line of a multi-line secret — or that line's
    // base64 — must be censored as well.
    let line = "kubeconfig-fixture-9c21";
    let line_wire = STANDARD.encode(line);
    let events = [
        json!({"host": "fixture", "task": "Store the cluster admin kubeconfig", "result": value}),
        json!({"host": "fixture", "task": "A later task echoes the fetch", "result": {
            "msg": format!("cat gives {PLAINTEXT} and {wire}"),
        }}),
        json!({"host": "fixture", "task": "A later task echoes one line of the fetch", "result": {
            "msg": format!("grep gives {line} and {line_wire}"),
        }}),
    ];
    let mut stdout = Vec::new();
    let mut execution = execution;
    for event in &events {
        let redacted = execution.lookups.redact(event);
        stdout.extend_from_slice(format!("{redacted}\n").as_bytes());
        execution.event(event)?;
    }
    captured.push_str(&String::from_utf8(stdout)?);
    captured.push_str(&std::fs::read_to_string(&log)?);
    drop(std::fs::remove_file(&log));
    for form in derived_forms(PLAINTEXT, &wire) {
        assert!(
            !captured.contains(&form),
            "captured output or results carry plaintext: {form}"
        );
    }
    // The grep ran over real captured output, not an empty file.
    assert!(captured.contains("[REDACTED]"));
    assert!(captured.contains("changed"));
    Ok(())
}

/// A failed fetch is a failed task, not a controller error: the agent's
/// result carries no content, and the run reports the failure the way
/// every other failed task is reported, so the recap counts it and
/// `ignoreErrors`, a retry and a rescue see it. The store is never
/// contacted: a failed fetch has no content to store.
#[tokio::test]
async fn a_failed_fetch_reports_a_failed_task() -> Result<()> {
    let store = Arc::new(Store::default());
    let execution = execution(
        Lookups::new(Resolver::new(store.clone()).expect("resolver")),
        None,
    )?;
    let value = run_fetch_with_status(
        &execution,
        serde_json::to_vec(&json!({"error": "no such file"}))?,
        "failed",
    )
    .await?;
    assert_eq!(value["failed"], json!(true), "the task failed");
    assert_eq!(value["changed"], json!(false), "nothing was stored");
    assert!(
        value.get("content").is_none(),
        "a failed result carries no content: {value}"
    );
    assert_eq!(store.sets.load(Ordering::SeqCst), 0, "no store write");
    assert_eq!(store.gets.load(Ordering::SeqCst), 0, "no store read");
    Ok(())
}

/// Under the synthetic resolver `FetchSecret` never writes: the fetch
/// reports known-after-apply and unchanged, and the fake store counts no
/// contact at all — neither the write nor the read path is touched.
#[tokio::test]
async fn synthetic_fetch_reports_known_after_apply_without_writing() -> Result<()> {
    const PLAINTEXT: &str = "kubeconfig-fixture-content\n";
    let wire = STANDARD.encode(PLAINTEXT);
    let store = Arc::new(Store::default());
    let execution = execution(
        Lookups::new(Resolver::synthetic_with(store.clone()).expect("synthetic resolver")),
        None,
    )?;
    let value = run_fetch(&execution, serde_json::to_vec(&json!({"content": wire}))?).await?;
    assert_eq!(value["knownAfterApply"], json!(true), "known after apply");
    assert_eq!(value["changed"], json!(false), "unchanged");
    assert_eq!(value["failed"], json!(false));
    assert!(value.get("content").is_none(), "the content is stripped");
    assert_eq!(
        store.sets.load(Ordering::SeqCst),
        0,
        "the synthetic resolver wrote"
    );
    assert_eq!(
        store.gets.load(Ordering::SeqCst),
        0,
        "the synthetic resolver read the store"
    );
    assert!(!serde_json::to_string(&value)?.contains(&wire));
    Ok(())
}
