use super::*;
use tokio::sync::{mpsc, oneshot};

type Started = (bool, oneshot::Sender<()>);
pub(super) struct TestConnection {
    started: mpsc::UnboundedSender<Started>,
    pending: Option<(u64, oneshot::Receiver<()>)>,
}
impl TestConnection {
    pub(super) fn send(&mut self, envelope: &v1::Envelope) -> Result<()> {
        let Some(v1::envelope::Msg::Plan(plan)) = &envelope.msg else {
            anyhow::bail!("expected plan");
        };
        let task = plan.tasks.first().context("missing task")?;
        let (release, pending) = oneshot::channel();
        self.started
            .send((task.no_log, release))
            .map_err(|_| anyhow::anyhow!("observer closed"))?;
        self.pending = Some((task.task_id, pending));
        Ok(())
    }
    pub(super) async fn next(&mut self) -> Result<Option<v1::Event>> {
        let (id, pending) = self.pending.take().context("missing pending result")?;
        pending.await?;
        Ok(Some(v1::Event {
            msg: Some(v1::event::Msg::TaskResult(v1::TaskResult {
                task_id: id,
                status: "ok".into(),
                result_json: b"{}".to_vec(),
                ..Default::default()
            })),
        }))
    }
}

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
            jobs: 2,
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
        output: Arc::new(Output::new(OutputFormat::Json, None)?),
        lookups: Arc::new(Lookups {
            resolver: Resolver::synthetic(),
            values: Mutex::new(Vec::new()),
        }),
    })
}
async fn add(execution: &Execution, host: &str, started: mpsc::UnboundedSender<Started>) {
    execution.connections.lock().await.insert(
        host.to_owned(),
        Arc::new(tokio::sync::Mutex::new(Connection::Test(TestConnection {
            started,
            pending: None,
        }))),
    );
}
fn request(host: &str) -> Request {
    Request {
        host: host.into(),
        task: v1::RenderedTask {
            task_id: 1,
            module: "stat".into(),
            iterations: vec![v1::Iteration {
                params_json: br#"{"path":"/synthetic"}"#.to_vec(),
                ..Default::default()
            }],
            ..Default::default()
        },
    }
}
async fn start(execution: Execution, host: &'static str, sensitive: bool) -> Result<TrackedValue> {
    let mut execution = execution;
    execution.execute_tracked(request(host), sensitive).await
}

#[tokio::test]
async fn public_dispatches_share_gate() -> Result<()> {
    let execution = execution()?;
    let (sent, mut recv) = mpsc::unbounded_channel();
    add(&execution, "a", sent.clone()).await;
    add(&execution, "b", sent).await;
    let a = tokio::spawn(start(execution.clone(), "a", false));
    let b = tokio::spawn(start(execution, "b", false));
    let (private_a, release_a) =
        tokio::time::timeout(std::time::Duration::from_secs(2), recv.recv())
            .await?
            .context("first")?;
    let (private_b, release_b) =
        tokio::time::timeout(std::time::Duration::from_secs(2), recv.recv())
            .await?
            .context("second")?;
    assert!(!private_a && !private_b);
    release_a
        .send(())
        .map_err(|()| anyhow::anyhow!("first closed"))?;
    release_b
        .send(())
        .map_err(|()| anyhow::anyhow!("second closed"))?;
    assert!(!a.await??.is_secret());
    assert!(!b.await??.is_secret());
    Ok(())
}

#[tokio::test]
async fn private_dispatch_precedes_alias_read_and_marks_result() -> Result<()> {
    let execution = execution()?;
    let (sent, mut recv) = mpsc::unbounded_channel();
    add(&execution, "a", sent.clone()).await;
    add(&execution, "b", sent).await;
    let writer = tokio::spawn(start(execution.clone(), "a", true));
    let (private, release_writer) =
        tokio::time::timeout(std::time::Duration::from_secs(2), recv.recv())
            .await?
            .context("writer")?;
    assert!(private);
    assert!(execution.effect_gate.try_read().is_err());
    let reader = tokio::spawn(start(execution, "b", false));
    release_writer
        .send(())
        .map_err(|()| anyhow::anyhow!("writer closed"))?;
    let (private, release_reader) =
        tokio::time::timeout(std::time::Duration::from_secs(2), recv.recv())
            .await?
            .context("reader")?;
    assert!(private);
    release_reader
        .send(())
        .map_err(|()| anyhow::anyhow!("reader closed"))?;
    assert!(writer.await??.is_secret());
    assert!(reader.await??.is_secret());
    Ok(())
}
