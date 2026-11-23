use super::*;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::time::{Duration, Instant};

struct Agent(Child);
impl Drop for Agent {
    fn drop(&mut self) {
        // Each fixture owns its process group, including any synthetic sleep.
        let _status = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.0.id())])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _status = self.0.wait();
    }
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready() {
        assert!(Instant::now() < deadline, "signal fixture timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn exercise(signal: &str, number: i32, phase: &str) {
    let dir = temp_dir(&format!("default-{signal}-{phase}"));
    let saved = dir.join("saved");
    // Preserve an already committed ledger across abnormal process termination.
    let mut seed = spawn_agent(&dir);
    let mut input = seed.stdin.take().unwrap();
    let mut output = seed.stdout.take().unwrap();
    write_frame(&mut input, &hello("signal-seed", PROTO_VERSION)).unwrap();
    let _: v1::Event = read_frame(&mut output).unwrap().unwrap();
    write_frame(&mut input, &copy_plan(1, &saved, "saved", "signal-saved")).unwrap();
    assert_eq!(read_task_result(&mut output).status, "changed");
    finish_agent(&mut seed, &mut input);
    let ledger = std::fs::read(dir.join("ledger/ledger.json")).unwrap();

    let mut agent = Agent(
        Command::new(env!("CARGO_BIN_EXE_ruxel-agent"))
            .env("RUXEL_STATE_DIR", &dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap(),
    );
    let mut input = agent.0.stdin.take().unwrap();
    let mut output = agent.0.stdout.take().unwrap();
    write_frame(&mut input, &hello("signal-active", PROTO_VERSION)).unwrap();
    let _: v1::Event = read_frame(&mut output).unwrap().unwrap();
    match phase {
        "idle" => {}
        "partial-length" => input.write_all(&[0x80]).unwrap(),
        "partial-body" => input.write_all(&[10, 18]).unwrap(),
        "task" => {
            let marker = dir.join("started");
            // The readiness file proves the module child actually started.
            let params = serde_json::json!({"argv": ["sh", "-c", "printf ready > \"$1\"; exec sleep 30", "signal-fixture", marker]});
            let mut plan = plan_with_params(
                2,
                "command",
                serde_json::to_vec(&params).unwrap(),
                "signal-blocked",
            );
            if let Some(Msg::Plan(plan)) = &mut plan.msg {
                plan.tasks[0].no_log = true;
            }
            write_frame(&mut input, &plan).unwrap();
            wait_until(|| marker.exists());
        }
        _ => unreachable!(),
    }
    input.flush().unwrap();
    assert!(
        Command::new("kill")
            .args([signal, &agent.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let mut terminated = None;
    wait_until(|| {
        terminated = agent.0.try_wait().unwrap();
        terminated.is_some()
    });
    assert_eq!(terminated.unwrap().signal(), Some(number));
    drop(agent); // Clean synthetic children before permitting another run.
    assert_eq!(std::fs::read_to_string(saved).unwrap(), "saved");
    assert_eq!(
        std::fs::read(dir.join("ledger/ledger.json")).unwrap(),
        ledger
    );
    assert_reconnects(&dir, "signal-reconnected");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sigterm_and_sigint_keep_default_termination_even_during_blocked_tasks() {
    for (signal, number) in [("-TERM", 15), ("-INT", 2)] {
        for phase in ["idle", "partial-length", "partial-body", "task"] {
            exercise(signal, number, phase);
        }
    }
}
