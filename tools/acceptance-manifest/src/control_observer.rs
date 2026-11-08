use crate::{control_run, runner::Env, state};
use anyhow::{Context, Result, bail, ensure};
use ruxel_engine::{
    controller::{self, Backend, Mode, Options, Request},
    expression::{Expr, LookupResolver, provenance::TrackedValue},
    secrets::{Lookup, ProcessBackend, Resolver},
    transport::LocalConnection,
};
use ruxel_proto::v1;
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, time::Duration};

#[derive(Default)]
struct Observations {
    registered: Value,
    events: Vec<Value>,
    calls: Vec<Request>,
    evaluations: Vec<TrackedValue>,
    retries: u64,
}
struct Observer {
    connection: LocalConnection,
    observed: Observations,
    resolver: Resolver,
}
impl LookupResolver for Observer {
    fn resolve(&self, expr: &Expr) -> Result<Value> {
        let Expr::Lookup {
            provider,
            key,
            argv,
            ..
        } = expr
        else {
            bail!("unexpected lookup")
        };
        ensure!(
            provider == "pipe"
                && key.as_deref() == Some("printf synthetic")
                && argv == &["printf synthetic"],
            "unlisted fixture lookup"
        );
        Ok(json!(
            self.resolver
                .resolve(&Lookup::Pipe {
                    command: "printf synthetic".into()
                })
                .map_err(anyhow::Error::msg)?
        ))
    }
}
impl Backend for Observer {
    fn evaluate(&mut self, _: &Value, _: &Map<String, Value>) -> Result<Value> {
        bail!("unsupported opaque evaluation")
    }
    fn evaluate_layered_expression(
        &mut self,
        value: &Value,
        scope: &TrackedValue,
        layers: &[Map<String, Value>],
    ) -> Result<(TrackedValue, BTreeMap<String, TrackedValue>)> {
        if let Some(results) = scope.value.get("results") {
            self.observed.registered = results.clone();
        }
        let evaluated = controller::evaluate_standard_layered(self, value, scope, layers)?;
        self.observed.evaluations.push(evaluated.0.clone());
        Ok(evaluated)
    }
    async fn execute(&mut self, request: Request) -> Result<Value> {
        ensure!(request.host == "fixture", "unexpected fixture host");
        let id = request.task.task_id;
        if request.task.name == "Exercise retries and until" {
            self.observed.retries += 1;
        }
        self.connection
            .send(&v1::Envelope {
                msg: Some(v1::envelope::Msg::Plan(v1::Plan {
                    tasks: vec![request.task.clone()],
                    blobs_referenced: vec![],
                })),
            })
            .await?;
        self.observed.calls.push(request);
        let mut started = false;
        loop {
            let event = self
                .connection
                .next_event()
                .await?
                .context("agent closed")?;
            match event.msg {
                Some(v1::event::Msg::TaskStart(start)) => {
                    ensure!(!started && start.task_id == id, "agent start mismatch");
                    started = true;
                }
                Some(v1::event::Msg::TaskResult(result)) => {
                    ensure!(started && result.task_id == id, "agent result mismatch");
                    let mut value: Value = serde_json::from_slice(&result.result_json)?;
                    value["changed"] = json!(result.changed);
                    value["failed"] = json!(result.status == "failed");
                    return Ok(value);
                }
                Some(v1::event::Msg::Log(log)) => {
                    control_run::privacy(&log.message)?;
                }
                _ => bail!("unexpected agent event"),
            }
        }
    }
    fn event(&mut self, event: &Value) -> Result<()> {
        self.observed.events.push(event.clone());
        Ok(())
    }
}
fn verify(
    observed: &Observations,
    facts: &BTreeMap<String, TrackedValue>,
    phase: &str,
    preview: bool,
) -> Result<()> {
    let text = serde_json::to_string(&observed.events)?;
    control_run::privacy(&text)?;
    let mapped = observed
        .events
        .iter()
        .find(|event| event["task"] == "Execute mapped loop with label and environment")
        .context("missing mapped event")?;
    ensure!(
        mapped["result"]["censored"] == true && mapped["result"].get("results").is_none(),
        "mapped event was not censored"
    );
    if !preview {
        let rows = observed.registered["fixture_loop"]["results"]
            .as_array()
            .context("missing registered loop")?;
        ensure!(
            rows.iter()
                .map(|row| row["stdout"].as_str())
                .collect::<Vec<_>>()
                == vec![Some("alpha:one"), Some("alpha:two"), Some("beta:three")],
            "registered loop mismatch"
        );
        let retry = &observed.registered["fixture_retry"];
        ensure!(
            retry["rc"] == 0
                && retry["attempts"] == observed.retries
                && observed.retries == if phase == "converged" { 1 } else { 2 },
            "retry result mismatch"
        );
        ensure!(
            facts.get("fixture").context("missing fixture facts")?.value["fixture_rescued"] == true,
            "rescue fact mismatch"
        );
    }
    verify_controls(observed, preview)
}
fn verify_controls(observed: &Observations, preview: bool) -> Result<()> {
    let count = |name: &str| {
        observed
            .calls
            .iter()
            .filter(|request| request.task.name == name)
            .count()
    };
    ensure!(
        count("Exercise nested mapping scalar variants") == 2
            && count("Execute mapped loop with label and environment") == 3
            && count("Exercise ignored failure") == 1
            && count("Trigger synthetic rescue") == 1
            && count("Exercise check-mode string value") == 1,
        "control calls mismatch"
    );
    ensure!(
        observed
            .calls
            .iter()
            .all(|request| request.task.force_check_mode == preview),
        "check mode mismatch"
    );
    ensure!(
        observed
            .evaluations
            .iter()
            .any(|v| v.value["msg"] == "nested-count=3")
            && observed
                .evaluations
                .iter()
                .any(|v| v.value["msg"] == "pairs=3 pipe=synthetic" && v.is_secret()),
        "interpolation mismatch"
    );
    let event = |name: &str| {
        observed
            .events
            .iter()
            .find(|event| event["task"] == name)
            .context("missing control event")
    };
    ensure!(
        event("Exercise indexed equality and boolean conjunction")?["result"]["msg"]
            == "first=alpha",
        "conditional result mismatch"
    );
    ensure!(
        event("Exercise exact expression nested in a literal loop")?["result"]["censored"] == true,
        "nested loop was not censored"
    );
    ensure!(
        event("Exercise literal assertion failure message")?["result"]["failed"] == false,
        "assertion mismatch"
    );
    let pause = event("Exercise interactive pause")?;
    ensure!(
        pause["result"]["changed"] == false
            && pause["result"]["skipped"].as_bool().unwrap_or(false) == preview,
        "pause result mismatch"
    );
    ensure!(
        observed
            .events
            .iter()
            .filter(|event| event.get("pause").is_some())
            .count()
            == usize::from(!preview),
        "pause event mismatch"
    );
    ensure!(
        observed
            .calls
            .iter()
            .filter(|request| request.task.name == "Exercise check-mode string value")
            .all(|request| request.task.check_mode_override),
        "check override mismatch"
    );
    Ok(())
}
pub(super) fn execute(env: &Env) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let config = ruxel_engine::model::validate_config(&ruxel_config::eval(&env.module)?)?;
        for phase in ["fresh", "converged", "drift_repair", "check", "diff"] {
            if matches!(phase, "drift_repair" | "check") {
                control_run::drift(&env.payload)?;
            }
            let preview = matches!(phase, "check" | "diff");
            let before = state::snapshot(&env.payload)?;
            let fixture = state::fixture_snapshot(env.root.path())?;
            let mode = match phase {
                "check" => Mode::Check,
                "diff" => Mode::Diff,
                _ => Mode::Apply,
            };
            let (connection, _, _) = LocalConnection::connect(
                &env.agent,
                &env.root.path().join("state"),
                phase,
                preview,
                phase == "diff",
                true,
            )
            .await?;
            let mut observer = Observer {
                connection,
                observed: Observations::default(),
                resolver: Resolver::new(std::sync::Arc::new(ProcessBackend::default())),
            };
            let mut facts = BTreeMap::new();
            let recap = tokio::time::timeout(
                Duration::from_secs(45),
                controller::run(
                    &config,
                    &BTreeMap::from([("fixture".into(), json!({}))]),
                    &Options {
                        mode,
                        ..Options::default()
                    },
                    &mut observer,
                    &mut facts,
                ),
            )
            .await??;
            ensure!(
                recap["fixture"].failed == 0
                    && recap["fixture"].rescued == u64::from(!preview)
                    && recap["fixture"].ignored == u64::from(!preview),
                "observer recap mismatch"
            );
            verify(&observer.observed, &facts, phase, preview)?;
            tokio::time::timeout(Duration::from_secs(5), observer.connection.shutdown()).await??;
            if preview {
                ensure!(
                    state::snapshot(&env.payload)? == before
                        && state::fixture_snapshot(env.root.path())? == fixture,
                    "observer preview mutated fixture"
                );
            } else {
                control_run::files(&env.payload)?;
            }
            control_run::privacy_state(&env.root.path().join("state"))?;
        }
        Ok(())
    })
}
