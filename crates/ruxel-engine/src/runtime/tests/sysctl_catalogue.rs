//! The sysctl catalogue is sent once per agent session: the plan's
//! declarations rode every frame, so N unconditional sysctls cost
//! O(N^2) wire and allocation across the session's frames and could
//! trip the frame limit — while the agent accumulates every frame's
//! set into its own plan set, so each entry travels exactly once.

use super::*;
use ruxel_proto::v1;

fn catalogue_execution() -> Result<Execution> {
    Ok(Execution {
        connection_failures: 0,
        conservative_sensitive: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        effect_gate: Arc::new(tokio::sync::RwLock::new(())),
        output: Arc::new(Output::new(OutputFormat::Json, None)?),
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
            workflow_namespace: String::new(),
        },
        connections: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        resources: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        sysctl_sent: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        source_gate: Arc::new(tokio::sync::RwLock::new(())),
        template_root: Arc::new(templates::SourceRoot::open(std::path::Path::new("."))?),
        copy_root: Arc::new(copy_sources::SourceRoot::open(std::path::Path::new("."))?),
        lookups: Arc::new(Lookups::new(
            Resolver::synthetic().expect("synthetic resolver"),
        )),
    })
}

fn decl(file: &str, value: &str) -> v1::SysctlDeclaration {
    v1::SysctlDeclaration {
        file: file.into(),
        name: "net.test".into(),
        value: value.into(),
    }
}

fn request(host: &str, catalogue: Vec<v1::SysctlDeclaration>) -> Request {
    Request {
        host: host.into(),
        task: v1::RenderedTask::default(),
        content_fingerprint: None,
        plan_sysctl: catalogue,
    }
}

#[tokio::test]
async fn the_catalogue_travels_once_per_agent_session() -> Result<()> {
    // Thread 345: the first frame carries the whole catalogue, so an
    // earlier task's preflight weighs the later stages' files before
    // they execute — and later frames carry nothing the session has
    // already seen, instead of resending N declarations per task.
    let execution = catalogue_execution()?;
    let catalogue = vec![
        decl("/etc/sysctl.d/10-a.conf", "1"),
        decl("/etc/sysctl.d/99-b.conf", "2"),
    ];
    assert_eq!(
        execution
            .unsent_sysctl(&request("h", catalogue.clone()))
            .await,
        catalogue,
        "the first frame carries the whole catalogue"
    );
    assert!(
        execution
            .unsent_sysctl(&request("h", catalogue.clone()))
            .await
            .is_empty(),
        "a repeat of the catalogue travels nothing"
    );
    Ok(())
}

#[tokio::test]
async fn later_declarations_join_later_frames_and_other_hosts_start_whole() -> Result<()> {
    // A declaration the plan gains after the first frame — a delegated
    // task's own static set, or a later stage's — still reaches the
    // agent on the next frame, and a session on another host starts
    // with the whole catalogue again.
    let execution = catalogue_execution()?;
    let catalogue = vec![decl("/etc/sysctl.d/10-a.conf", "1")];
    assert_eq!(
        execution
            .unsent_sysctl(&request("h", catalogue.clone()))
            .await,
        catalogue,
        "the first frame carries the whole catalogue"
    );
    let grown = vec![
        decl("/etc/sysctl.d/10-a.conf", "1"),
        decl("/etc/sysctl.d/99-b.conf", "2"),
    ];
    assert_eq!(
        execution.unsent_sysctl(&request("h", grown.clone())).await,
        vec![decl("/etc/sysctl.d/99-b.conf", "2")],
        "only the unseen entry rides the later frame"
    );
    assert_eq!(
        execution
            .unsent_sysctl(&request("other", grown.clone()))
            .await,
        grown,
        "another agent session starts with the whole catalogue"
    );
    Ok(())
}
