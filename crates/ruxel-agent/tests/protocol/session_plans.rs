//! One session's per-task Plan frames share the plan state: the
//! engine frames every task as its own Plan, so the second frame's
//! tasks resolve against the first frame's declarations — and a second
//! apply replays its later stage through the ledger's carry-over while
//! the records it drops expire at the plan's end.

use super::{finish_agent, hello, plan_with_params, read_task_result, spawn_agent, temp_dir};
use ruxel_proto::{
    PROTO_VERSION,
    frame::{read_frame, write_frame},
    v1::{self, envelope::Msg},
};

fn check_hello(run_id: &str) -> v1::Envelope {
    let mut greeting = hello(run_id, PROTO_VERSION);
    let Some(Msg::Hello(ref mut hello)) = greeting.msg else {
        unreachable!();
    };
    hello.check_mode = true;
    greeting
}

/// The sysctl files the stored records still declare: records are
/// keyed by workflow and task, so the read crosses the per-workflow
/// maps first.
fn managed_files(ledger: &serde_json::Value) -> Vec<String> {
    let mut files: Vec<String> = ledger
        .as_object()
        .unwrap()
        .values()
        .flat_map(|owned| {
            owned
                .as_object()
                .unwrap()
                .values()
                .cloned()
                .collect::<Vec<_>>()
        })
        .filter_map(|record| {
            record
                .get("managed_sysctl")?
                .get("file")?
                .as_str()
                .map(str::to_string)
        })
        .collect();
    files.sort();
    files
}

fn sysctl_params(file: &std::path::Path, value: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "name": "net.ipv4.ruxel_session_key",
        "value": value,
        "sysctl_file": file.to_string_lossy(),
        "sysctl_set": false,
        "reload": false,
    }))
    .unwrap()
}

#[test]
fn check_mode_group_then_user_shares_the_planned_group() {
    // Two per-task Plan frames in check mode: the user frame resolves
    // its primary group against the group frame's planned declaration.
    // Resetting the plan state on every Plan frame fails the user task
    // with "group not found" instead.
    let dir = temp_dir("session-chain");
    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();
    write_frame(&mut stdin, &check_hello("session-chain")).unwrap();
    let _: v1::Event = read_frame(&mut stdout).unwrap().expect("hello ack");

    let group_params =
        serde_json::to_vec(&serde_json::json!({"name": "ruxel-sess-group"})).unwrap();
    write_frame(
        &mut stdin,
        &plan_with_params(1, "group", group_params, "sess-group"),
    )
    .unwrap();
    assert_eq!(read_task_result(&mut stdout).status, "changed");

    let user_params = serde_json::to_vec(&serde_json::json!({
        "name": "ruxel-sess-user",
        "group": "ruxel-sess-group",
    }))
    .unwrap();
    write_frame(
        &mut stdin,
        &plan_with_params(2, "user", user_params, "sess-user"),
    )
    .unwrap();
    let user = read_task_result(&mut stdout);
    let body: serde_json::Value = serde_json::from_slice(&user.result_json).unwrap();
    assert_eq!(
        user.status, "changed",
        "the later frame resolves the planned group: {body}"
    );
    finish_agent(&mut agent, &mut stdin);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn second_apply_replays_through_the_fallback_and_drops_expire() {
    // The first apply declares one key in two files; the second apply,
    // with fresh agent state, replays only the later stage at a new
    // value. The preflight admits it through the ledger's carry-over,
    // and the plan's end expires the dropped first stage: exactly one
    // run of residual remains on the fallback.
    let dir = temp_dir("session-second-apply");
    let first = dir.join("10-first.conf");
    let second = dir.join("99-second.conf");

    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();
    write_frame(&mut stdin, &hello("session-first", PROTO_VERSION)).unwrap();
    let _: v1::Event = read_frame(&mut stdout).unwrap().expect("hello ack");
    write_frame(
        &mut stdin,
        &plan_with_params(1, "sysctl", sysctl_params(&first, "1"), "sess-first"),
    )
    .unwrap();
    assert_eq!(read_task_result(&mut stdout).status, "changed");
    write_frame(
        &mut stdin,
        &plan_with_params(2, "sysctl", sysctl_params(&second, "2"), "sess-second"),
    )
    .unwrap();
    assert_eq!(read_task_result(&mut stdout).status, "changed");
    finish_agent(&mut agent, &mut stdin);
    assert_eq!(
        std::fs::read_to_string(&first).unwrap().trim(),
        "net.ipv4.ruxel_session_key=1"
    );
    assert_eq!(
        std::fs::read_to_string(&second).unwrap().trim(),
        "net.ipv4.ruxel_session_key=2"
    );

    let mut rerun = spawn_agent(&dir);
    let mut rerun_stdin = rerun.stdin.take().unwrap();
    let mut rerun_stdout = rerun.stdout.take().unwrap();
    write_frame(&mut rerun_stdin, &hello("session-second", PROTO_VERSION)).unwrap();
    let _: v1::Event = read_frame(&mut rerun_stdout).unwrap().expect("hello ack");
    write_frame(
        &mut rerun_stdin,
        &plan_with_params(
            3,
            "sysctl",
            sysctl_params(&second, "3"),
            "sess-second-again",
        ),
    )
    .unwrap();
    let replayed = read_task_result(&mut rerun_stdout);
    assert_eq!(
        replayed.status, "changed",
        "the replayed later stage is admitted through the carry-over"
    );
    finish_agent(&mut rerun, &mut rerun_stdin);
    assert_eq!(
        std::fs::read_to_string(&second).unwrap().trim(),
        "net.ipv4.ruxel_session_key=3"
    );

    let ledger: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("ledger/ledger.json")).unwrap()).unwrap();
    assert_eq!(
        managed_files(&ledger),
        vec![second.to_string_lossy().into_owned()],
        "the dropped first stage expires at the plan's end"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
