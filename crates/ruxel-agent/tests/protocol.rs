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
