use super::*;
use crate::expression::{Expr, LookupResolver};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A controller-side store that counts every fnox read and can be set to
/// fail, as a deleted fnox value would.
struct CountingStore {
    reads: AtomicUsize,
    works: bool,
    commands: Vec<Value>,
}

impl LookupResolver for CountingStore {
    fn resolve(&self, expr: &Expr) -> Result<Value> {
        let crate::expression::Expr::Lookup { provider, .. } = expr else {
            bail!("unexpected expression lookup");
        };
        anyhow::ensure!(provider == "fnox", "unexpected lookup provider {provider}");
        self.reads.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(self.works, "fnox store has no APP_INIT_TOKEN");
        Ok(Value::String("token-value".into()))
    }
}

impl Backend for CountingStore {
    fn evaluate(&mut self, _: &Value, _: &Map<String, Value>) -> Result<Value> {
        bail!("unexpected opaque evaluation")
    }
    fn evaluate_layered_expression(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        layers: &[Map<String, Value>],
    ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
        evaluate_standard_layered(self, value, scope, layers)
    }
    fn execute(&mut self, request: Request) -> impl Future<Output = Result<Value>> {
        let params: Value =
            serde_json::from_slice(&request.task.iterations[0].params_json).expect("params");
        let commands = &mut self.commands;
        std::future::ready((|| {
            // The agent stats the path for real: the guard reads the host.
            match request.task.module.as_str() {
                "stat" => {
                    let path = params["path"].as_str().expect("stat path");
                    Ok(
                        json!({"stat": {"exists": std::fs::metadata(path).is_ok()}, "changed": false, "failed": false}),
                    )
                }
                "command" => {
                    commands.push(params.clone());
                    Ok(json!({"changed": true, "failed": false, "rc": 0}))
                }
                other => bail!("unexpected module {other}"),
            }
        })())
    }
    fn event(&mut self, _: &Value) -> Result<()> {
        Ok(())
    }
}

fn marker() -> std::path::PathBuf {
    // The fixture names this literal path; /tmp is shared by macOS and Linux.
    std::path::PathBuf::from("/tmp/ruxel-creates-skip-fixture/marker")
}

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow"
    ))
    .join(name)
}

fn fixture_config() -> Result<Value> {
    let options = ruxel_config::EvalOptions {
        root_dir: Some(fixture("")),
        ..ruxel_config::EvalOptions::default()
    };
    let evaluated = ruxel_config::eval_config(&fixture("valid/secret-creates-skip.pkl"), &options)
        .map_err(|error| anyhow::anyhow!("fixture evaluation failed: {error}"))?;
    crate::model::validate_config(&evaluated).context("fixture validation failed")
}

/// One run, both sides of the guard: the created path gates the task before
/// its secret environment resolves — the store is never read, so a deleted
/// fnox value cannot fail a skipped action — and an absent path still
/// resolves the environment and runs.
#[tokio::test]
async fn a_created_path_gates_the_task_before_its_secret_environment() -> Result<()> {
    std::fs::create_dir_all(marker().parent().expect("marker parent"))?;

    // The marker exists and the store is broken: the run skips cleanly.
    std::fs::write(marker(), b"done")?;
    let mut backend = CountingStore {
        reads: AtomicUsize::new(0),
        works: false,
        commands: Vec::new(),
    };
    let recap = run(
        &fixture_config()?,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.reads.load(Ordering::SeqCst), 0, "store reads");
    assert!(backend.commands.is_empty(), "command must not run");
    // The probe ran; the guarded command was skipped.
    assert_eq!(recap["worker-1"].ok, 1);
    assert_eq!(recap["worker-1"].skipped, 1);
    assert_eq!(recap["worker-1"].failed, 0);
    assert_eq!(recap["worker-1"].changed, 0);

    // The marker is absent: the environment resolves and the command runs.
    std::fs::remove_file(marker())?;
    let mut backend = CountingStore {
        reads: AtomicUsize::new(0),
        works: true,
        commands: Vec::new(),
    };
    let recap = run(
        &fixture_config()?,
        &BTreeMap::new(),
        &Options::default(),
        &mut backend,
        &mut BTreeMap::new(),
    )
    .await?;
    assert_eq!(backend.reads.load(Ordering::SeqCst), 1, "store reads");
    assert_eq!(backend.commands.len(), 1);
    assert_eq!(recap["worker-1"].ok, 2);
    assert_eq!(recap["worker-1"].changed, 1);
    assert_eq!(recap["worker-1"].failed, 0);
    Ok(())
}
