use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Fixture {
    root: PathBuf,
    agent: PathBuf,
}
#[derive(Default)]
struct Behavior {
    missing: bool,
    shutdown_failure: bool,
    delegated_failure: bool,
    task_response: Option<&'static str>,
}
impl Fixture {
    fn new() -> Result<Self> {
        // Parallel tests can read the same clock value, so a counter keeps
        // each fixture directory distinct.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ruxel-exit-classification-{}-{unique}-{sequence}",
            std::process::id()
        ));
        let fixture = Self {
            agent: root.join("fixture-agent"),
            root,
        };
        fs::create_dir(&fixture.root)?;
        let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2024", "--crate-name", "local_exit_agent"])
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/local_exit_agent.rs"
            ))
            .arg("-o")
            .arg(&fixture.agent)
            .output()?;
        assert!(compiled.status.success(), "{compiled:?}");
        Ok(fixture)
    }

    fn run(&self, operation: &str, controls: &str, behavior: &Behavior) -> Result<Output> {
        let module = self.root.join("config.pkl");
        fs::write(
            &module,
            format!(
                r#"import "modulepath:/ruxel/Ruxel.pkl"
output {{ value = new Ruxel.Config {{
 inventory {{ hosts {{ ["fixture"] {{ address = "fixture.invalid" }} }} }}
 plays {{ new {{ name = "exit classification"; targets {{ "fixture" }}; tasks {{
  new {{ name = "controller task"; {controls} operation = {operation} }}
 }} }} }}
}} }}
"#
            ),
        )?;
        let mut command = Command::new(env!("CARGO_BIN_EXE_ruxel"));
        command
            .arg("apply")
            .arg(&module)
            .args(["--local", "--state-dir"])
            .arg(self.root.join("state"))
            .arg("--agent")
            .arg(if behavior.missing {
                self.root.join("missing-agent")
            } else {
                self.agent.clone()
            })
            .env_remove("RUXEL_TEST_SHUTDOWN_FAILURE")
            .env_remove("RUXEL_TEST_DELEGATE_FAILURE")
            .env_remove("RUXEL_TEST_TASK_RESPONSE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if behavior.shutdown_failure {
            command.env("RUXEL_TEST_SHUTDOWN_FAILURE", "1");
        }
        if behavior.delegated_failure {
            command.env("RUXEL_TEST_DELEGATE_FAILURE", "1");
        }
        if let Some(response) = behavior.task_response {
            command.env("RUXEL_TEST_TASK_RESPONSE", response);
        }
        let mut child = command.spawn()?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while child.try_wait()?.is_none() {
            if Instant::now() >= deadline {
                child.kill()?;
                child.wait()?;
                return Err("fixture CLI timed out".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(child.wait_with_output()?)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _result = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn local_connection_and_shutdown_failures_are_distinct_from_task_failures() -> Result<()> {
    let fixture = Fixture::new()?;
    for (operation, missing, shutdown_failure, expected, unreachable) in [
        ("new Ruxel.Debug { msg = \"ok\" }", false, false, 0, false),
        (
            "new Ruxel.Fail { msg = \"task failed\" }",
            true,
            false,
            7,
            true,
        ),
        (
            "new Ruxel.Fail { msg = \"task failed\" }",
            false,
            false,
            8,
            false,
        ),
        ("new Ruxel.Debug { msg = \"ok\" }", false, true, 7, true),
        (
            "new Ruxel.Fail { msg = \"task failed\" }",
            false,
            true,
            7,
            true,
        ),
    ] {
        let output = fixture.run(
            operation,
            "",
            &Behavior {
                missing,
                shutdown_failure,
                ..Behavior::default()
            },
        )?;
        assert_eq!(output.status.code(), Some(expected), "{output:?}");
        let events: Vec<Value> = std::str::from_utf8(&output.stdout)?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let recaps: Vec<_> = events
            .iter()
            .filter_map(|event| event.get("recap"))
            .collect();
        assert_eq!(recaps.len(), 1, "{events:?}");
        assert_eq!(recaps[0]["failed"], u64::from(expected != 0));
        assert_eq!(recaps[0].get("unreachable").is_some(), unreachable);
        if unreachable {
            assert_eq!(recaps[0]["unreachable"], 1);
        }
        assert_eq!(
            events
                .iter()
                .any(|event| event["task"] == "controller task"),
            !missing,
            "preconnection failure must prevent task execution"
        );
    }
    Ok(())
}

#[test]
fn delegated_connection_failure_survives_sensitive_error_redaction() -> Result<()> {
    let fixture = Fixture::new()?;
    let output = fixture.run(
        "new Ruxel.Command { argv { \"unused\" } }",
        "delegateTo = \"localhost\"; delegateAllow { \"localhost\" }; delegateLocalhost = true; noLog = true;",
        &Behavior {
            delegated_failure: true,
            ..Behavior::default()
        },
    )?;
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    let events: Vec<Value> = std::str::from_utf8(&output.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    let recaps: Vec<_> = events
        .iter()
        .filter_map(|event| event.get("recap"))
        .collect();
    assert_eq!(recaps.len(), 1, "{events:?}");
    assert_eq!(recaps[0]["unreachable"], 1);
    assert_eq!(recaps[0]["failed"], 1);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic delegated"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic delegated"));
    Ok(())
}

#[test]
fn task_result_failure_is_not_a_transport_disconnect() -> Result<()> {
    for (response, expected) in [("failed", 8), ("disconnect", 7)] {
        let fixture = Fixture::new()?;
        let output = fixture.run(
            "new Ruxel.Command { argv { \"unused\" } }",
            "",
            &Behavior {
                task_response: Some(response),
                ..Behavior::default()
            },
        )?;
        assert_eq!(output.status.code(), Some(expected), "{output:?}");
        let events: Vec<Value> = std::str::from_utf8(&output.stdout)?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let recap = events
            .iter()
            .find_map(|event| event.get("recap"))
            .ok_or("missing recap")?;
        assert_eq!(recap["failed"], 1);
        assert_eq!(recap.get("unreachable").is_some(), response == "disconnect");
        if response == "disconnect" {
            assert_eq!(recap["unreachable"], 1);
        }
    }
    Ok(())
}
