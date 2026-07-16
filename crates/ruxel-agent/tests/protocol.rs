//! Agent protocol-loop tests against the real binary over pipes — the
//! transport-free half of the M2 gate: handshake + facts + clean shutdown,
//! version mismatch, EOF resilience, the single-run lock, and kill -9
//! leaving the state dir reusable.

use ruxel_proto::PROTO_VERSION;
use ruxel_proto::frame::{read_frame, write_frame};
use ruxel_proto::v1::{self, envelope::Msg};
use std::process::{Child, Command, Stdio};

fn spawn_agent(state_dir: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_ruxel-agent"))
        .env("RUXEL_STATE_DIR", state_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("agent spawns")
}

fn hello(run_id: &str, proto_version: u32) -> v1::Envelope {
    v1::Envelope {
        msg: Some(Msg::Hello(v1::Hello {
            proto_version,
            run_id: run_id.into(),
            ..Default::default()
        })),
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ruxel-agent-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_plan(
    task_id: u64,
    dest: &std::path::Path,
    content: &str,
    ledger_key: &str,
) -> v1::Envelope {
    let params_json =
        serde_json::to_vec(&serde_json::json!({"dest": dest, "content": content})).unwrap();
    plan_with_params(task_id, "copy", params_json, ledger_key)
}

fn plan_with_params(
    task_id: u64,
    module: &str,
    params_json: Vec<u8>,
    ledger_key: &str,
) -> v1::Envelope {
    v1::Envelope {
        msg: Some(Msg::Plan(v1::Plan {
            tasks: vec![v1::RenderedTask {
                task_id,
                name: format!("synthetic {module}"),
                module: module.into(),
                rendered: true,
                iterations: vec![v1::Iteration {
                    params_json,
                    ledger_key: ledger_key.into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        })),
    }
}

fn read_task_result(stdout: &mut impl std::io::Read) -> v1::TaskResult {
    let start: v1::Event = read_frame(stdout).unwrap().expect("task start");
    assert!(matches!(start.msg, Some(v1::event::Msg::TaskStart(_))));
    let event: v1::Event = read_frame(stdout).unwrap().expect("task result");
    let Some(v1::event::Msg::TaskResult(result)) = event.msg else {
        panic!("expected TaskResult, got {event:?}")
    };
    result
}

fn finish_agent(agent: &mut Child, stdin: &mut impl std::io::Write) {
    write_frame(
        stdin,
        &v1::Envelope {
            msg: Some(Msg::Done(v1::Done {})),
        },
    )
    .unwrap();
    assert_eq!(agent.wait().unwrap().code(), Some(0));
}

#[test]
fn handshake_facts_clean_shutdown() {
    let dir = temp_dir("handshake");
    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();

    write_frame(&mut stdin, &hello("t1", PROTO_VERSION)).unwrap();
    let event: v1::Event = read_frame(&mut stdout).unwrap().expect("an event");
    let Some(v1::event::Msg::HelloAck(ack)) = event.msg else {
        panic!("expected HelloAck, got {event:?}");
    };
    assert_eq!(ack.proto_version, PROTO_VERSION);
    assert_eq!(ack.agent_version, env!("CARGO_PKG_VERSION"));
    let facts = ack.facts.expect("facts present");
    assert!(!facts.architecture.is_empty());

    write_frame(
        &mut stdin,
        &v1::Envelope {
            msg: Some(Msg::Done(v1::Done {})),
        },
    )
    .unwrap();
    let status = agent.wait().unwrap();
    assert_eq!(status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn proto_version_mismatch_refused() {
    let dir = temp_dir("mismatch");
    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();

    write_frame(&mut stdin, &hello("t2", PROTO_VERSION + 1)).unwrap();
    let event: v1::Event = read_frame(&mut stdout).unwrap().expect("an event");
    assert!(
        matches!(event.msg, Some(v1::event::Msg::Log(l)) if l.message.contains("mismatch")),
        "expected mismatch log"
    );
    assert_eq!(agent.wait().unwrap().code(), Some(65));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn eof_without_done_exits_clean() {
    let dir = temp_dir("eof");
    let mut agent = spawn_agent(&dir);
    let stdin = agent.stdin.take().unwrap();
    drop(stdin); // controller vanished
    assert_eq!(agent.wait().unwrap().code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ledger_flushes_on_eof_not_just_done() {
    let dir = temp_dir("ledger-eof");
    let dest = dir.join("managed-file");
    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();

    write_frame(&mut stdin, &hello("ledger-eof", PROTO_VERSION)).unwrap();
    let ack: v1::Event = read_frame(&mut stdout).unwrap().expect("hello ack");
    assert!(matches!(ack.msg, Some(v1::event::Msg::HelloAck(_))));

    let params = serde_json::to_vec(&serde_json::json!({
        "dest": dest,
        "content": "durable"
    }))
    .unwrap();
    write_frame(
        &mut stdin,
        &v1::Envelope {
            msg: Some(Msg::Plan(v1::Plan {
                tasks: vec![v1::RenderedTask {
                    task_id: 1,
                    name: "cacheable copy".into(),
                    module: "copy".into(),
                    rendered: true,
                    iterations: vec![v1::Iteration {
                        params_json: params,
                        ledger_key: "ledger-eof-key".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            })),
        },
    )
    .unwrap();

    let start: v1::Event = read_frame(&mut stdout).unwrap().expect("task start");
    assert!(matches!(start.msg, Some(v1::event::Msg::TaskStart(_))));
    let result: v1::Event = read_frame(&mut stdout).unwrap().expect("task result");
    assert!(matches!(result.msg, Some(v1::event::Msg::TaskResult(ref r)) if r.status == "changed"));

    drop(stdin);
    assert_eq!(agent.wait().unwrap().code(), Some(0));
    let ledger = dir.join("ledger/ledger.json");
    assert!(ledger.exists());
    assert!(!std::fs::read(&ledger).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_log_copy_emits_no_diff_and_leaves_no_ledger_record() {
    const SYNTHETIC_CONTENT: &str = "test-content-not-a-secret";

    let dir = temp_dir("no-log");
    let dest = dir.join("managed-file");
    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();

    let mut greeting = hello("no-log", PROTO_VERSION);
    let Some(Msg::Hello(ref mut hello)) = greeting.msg else {
        unreachable!();
    };
    hello.diff_mode = true;
    write_frame(&mut stdin, &greeting).unwrap();
    let _: v1::Event = read_frame(&mut stdout).unwrap().expect("hello ack");

    let params = serde_json::to_vec(&serde_json::json!({
        "dest": dest,
        "content": SYNTHETIC_CONTENT
    }))
    .unwrap();
    write_frame(
        &mut stdin,
        &v1::Envelope {
            msg: Some(Msg::Plan(v1::Plan {
                tasks: vec![v1::RenderedTask {
                    task_id: 1,
                    name: "private copy".into(),
                    module: "copy".into(),
                    rendered: true,
                    no_log: true,
                    iterations: vec![v1::Iteration {
                        params_json: params,
                        ledger_key: "no-log-key".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            })),
        },
    )
    .unwrap();

    let _: v1::Event = read_frame(&mut stdout).unwrap().expect("task start");
    let event: v1::Event = read_frame(&mut stdout).unwrap().expect("task result");
    let Some(v1::event::Msg::TaskResult(result)) = event.msg else {
        panic!("expected TaskResult");
    };
    assert!(result.diff.is_empty());
    assert!(!String::from_utf8_lossy(&result.result_json).contains(SYNTHETIC_CONTENT));

    drop(stdin);
    assert_eq!(agent.wait().unwrap().code(), Some(0));
    assert!(!dir.join("ledger/ledger.json").exists());
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), SYNTHETIC_CONTENT);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn second_agent_is_locked_out_and_kill9_releases() {
    let dir = temp_dir("lock");

    // First agent holds the lock mid-run.
    let mut first = spawn_agent(&dir);
    let mut stdin1 = first.stdin.take().unwrap();
    let mut stdout1 = first.stdout.take().unwrap();
    write_frame(&mut stdin1, &hello("run1", PROTO_VERSION)).unwrap();
    let _ack: v1::Event = read_frame(&mut stdout1).unwrap().expect("ack");

    // Second agent must refuse (exit 66).
    let mut second = spawn_agent(&dir);
    let _stdin2 = second.stdin.take().unwrap();
    let mut stdout2 = second.stdout.take().unwrap();
    let event: v1::Event = read_frame(&mut stdout2).unwrap().expect("an event");
    assert!(
        matches!(event.msg, Some(v1::event::Msg::Log(l)) if l.message.contains("lock")),
        "expected lock-held log"
    );
    assert_eq!(second.wait().unwrap().code(), Some(66));

    // kill -9 the first: the OS releases the lock; a rerun succeeds
    // (the M2 gate's disconnect-mid-stream reusability).
    first.kill().unwrap();
    let _ = first.wait();

    let mut third = spawn_agent(&dir);
    let mut stdin3 = third.stdin.take().unwrap();
    let mut stdout3 = third.stdout.take().unwrap();
    write_frame(&mut stdin3, &hello("run3", PROTO_VERSION)).unwrap();
    let event: v1::Event = read_frame(&mut stdout3).unwrap().expect("an event");
    assert!(matches!(event.msg, Some(v1::event::Msg::HelloAck(_))));
    write_frame(
        &mut stdin3,
        &v1::Envelope {
            msg: Some(Msg::Done(v1::Done {})),
        },
    )
    .unwrap();
    assert_eq!(third.wait().unwrap().code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plan_executes_and_returns_result() {
    let dir = temp_dir("plan-executes");
    let dest = dir.join("managed");
    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();
    write_frame(&mut stdin, &hello("plan-executes", PROTO_VERSION)).unwrap();
    let _: v1::Event = read_frame(&mut stdout).unwrap().expect("hello ack");
    write_frame(
        &mut stdin,
        &copy_plan(41, &dest, "first", "plan-executes-key"),
    )
    .unwrap();
    let result = read_task_result(&mut stdout);
    assert_eq!(
        (result.task_id, result.status.as_str(), result.changed),
        (41, "changed", true)
    );
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "first");
    finish_agent(&mut agent, &mut stdin);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ledger_replays_converged_task() {
    use std::os::unix::fs::MetadataExt;

    let dir = temp_dir("ledger-replay");
    let dest = dir.join("managed");
    let mut ledger_inode = None;
    for (run_id, expected_status, expected_changed) in
        [("first", "changed", true), ("second", "ok", false)]
    {
        let mut agent = spawn_agent(&dir);
        let mut stdin = agent.stdin.take().unwrap();
        let mut stdout = agent.stdout.take().unwrap();
        write_frame(&mut stdin, &hello(run_id, PROTO_VERSION)).unwrap();
        let _: v1::Event = read_frame(&mut stdout).unwrap().expect("hello ack");
        write_frame(
            &mut stdin,
            &copy_plan(42, &dest, "stable", "stable-copy-key"),
        )
        .unwrap();
        let result = read_task_result(&mut stdout);
        assert_eq!(result.status, expected_status);
        assert_eq!(result.changed, expected_changed);
        finish_agent(&mut agent, &mut stdin);
        let inode = std::fs::metadata(dir.join("ledger/ledger.json"))
            .unwrap()
            .ino();
        if let Some(first_inode) = ledger_inode {
            assert_eq!(
                inode, first_inode,
                "cache hit leaves the clean ledger unrewritten"
            );
        } else {
            ledger_inode = Some(inode);
        }
    }
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "stable");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_params_fail_without_killing_agent() {
    let dir = temp_dir("bad-params");
    let dest = dir.join("after-error");
    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();
    write_frame(&mut stdin, &hello("bad-params", PROTO_VERSION)).unwrap();
    let _: v1::Event = read_frame(&mut stdout).unwrap().expect("hello ack");

    write_frame(
        &mut stdin,
        &plan_with_params(51, "copy", b"{".to_vec(), "bad-key"),
    )
    .unwrap();
    let failed = read_task_result(&mut stdout);
    assert_eq!((failed.status.as_str(), failed.changed), ("failed", true));
    let body: serde_json::Value = serde_json::from_slice(&failed.result_json).unwrap();
    assert!(body["msg"].as_str().unwrap().contains("bad params"));

    write_frame(
        &mut stdin,
        &copy_plan(52, &dest, "alive", "after-error-key"),
    )
    .unwrap();
    let recovered = read_task_result(&mut stdout);
    assert_eq!(recovered.status, "changed");
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "alive");
    finish_agent(&mut agent, &mut stdin);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_task_halts_remaining_batch_when_requested() {
    let dir = temp_dir("halt-batch");
    let dest = dir.join("must-not-exist");
    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();
    write_frame(&mut stdin, &hello("halt-batch", PROTO_VERSION)).unwrap();
    let _: v1::Event = read_frame(&mut stdout).unwrap().expect("hello ack");

    let copy_params = serde_json::to_vec(&serde_json::json!({
        "dest": dest,
        "content": "wrong"
    }))
    .unwrap();
    let task = |task_id, params_json, halt_on_failure| v1::RenderedTask {
        task_id,
        name: format!("task {task_id}"),
        module: "copy".into(),
        rendered: true,
        iterations: vec![v1::Iteration {
            params_json,
            ledger_key: format!("halt-key-{task_id}"),
            ..Default::default()
        }],
        halt_on_failure,
        ..Default::default()
    };
    write_frame(
        &mut stdin,
        &v1::Envelope {
            msg: Some(Msg::Plan(v1::Plan {
                tasks: vec![task(61, b"{".to_vec(), true), task(62, copy_params, true)],
                ..Default::default()
            })),
        },
    )
    .unwrap();
    let failed = read_task_result(&mut stdout);
    assert_eq!(failed.task_id, 61);
    assert_eq!(failed.status, "failed");

    finish_agent(&mut agent, &mut stdin);
    assert!(
        !dest.exists(),
        "task after failed batch member must not run"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
