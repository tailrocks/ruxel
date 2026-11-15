use super::{hello, read_task_result, spawn_agent, temp_dir};
use ruxel_proto::{
    PROTO_VERSION,
    frame::{read_frame, write_frame},
    v1::{self, envelope::Msg},
};

fn finish(agent: &mut std::process::Child, stdin: &mut impl std::io::Write, filtered: bool) {
    write_frame(
        stdin,
        &v1::Envelope {
            msg: Some(Msg::Done(v1::Done { filtered })),
        },
    )
    .unwrap();
    assert_eq!(agent.wait().unwrap().code(), Some(0));
}

fn send_copy(
    stdin: &mut impl std::io::Write,
    stdout: &mut impl std::io::Read,
    task_id: u64,
    dest: &std::path::Path,
    content: &str,
    key: &str,
) -> v1::TaskResult {
    let params =
        serde_json::to_vec(&serde_json::json!({"dest": dest, "content": content})).unwrap();
    write_frame(
        stdin,
        &v1::Envelope {
            msg: Some(Msg::Plan(v1::Plan {
                tasks: vec![v1::RenderedTask {
                    task_id,
                    name: "filtered done probe".into(),
                    module: "copy".into(),
                    rendered: true,
                    iterations: vec![v1::Iteration {
                        params_json: params,
                        ledger_key: key.into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            })),
        },
    )
    .unwrap();
    read_task_result(stdout)
}

fn ledger_records(state_dir: &std::path::Path) -> usize {
    let ledger = std::fs::read(state_dir.join("ledger/ledger.json")).unwrap();
    let records: serde_json::Value = serde_json::from_slice(&ledger).unwrap();
    records.as_object().unwrap().len()
}

fn filtered_done_case(filtered: bool) {
    // A filtered run omits tasks the workflow keeps: its end must not
    // expire the records of the tasks it never sent, while a complete
    // unfiltered plan expires the tasks it dropped.
    let dir = temp_dir(if filtered {
        "done-filtered"
    } else {
        "done-full"
    });
    let dropped_a = dir.join("dropped-a");
    let dropped_b = dir.join("dropped-b");
    let kept = dir.join("kept");

    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();
    write_frame(&mut stdin, &hello("seed", PROTO_VERSION)).unwrap();
    let _: v1::Event = read_frame(&mut stdout).unwrap().expect("hello ack");
    for (task_id, dest, key) in [
        (1, dropped_a.clone(), "dropped-a-key"),
        (2, dropped_b.clone(), "dropped-b-key"),
    ] {
        let seeded = send_copy(&mut stdin, &mut stdout, task_id, &dest, "stable", key);
        assert_eq!(seeded.status, "changed");
    }
    finish(&mut agent, &mut stdin, false);
    assert_eq!(ledger_records(&dir), 2);

    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();
    write_frame(&mut stdin, &hello("tagged", PROTO_VERSION)).unwrap();
    let _: v1::Event = read_frame(&mut stdout).unwrap().expect("hello ack");
    let sent = send_copy(&mut stdin, &mut stdout, 3, &kept, "stable", "kept-key");
    assert_eq!(sent.status, "changed");
    finish(&mut agent, &mut stdin, filtered);
    if filtered {
        assert_eq!(
            ledger_records(&dir),
            3,
            "a filtered run must not expire the records it never sent"
        );
        // The surviving record is still valid cache, not just present.
        let mut agent = spawn_agent(&dir);
        let mut stdin = agent.stdin.take().unwrap();
        let mut stdout = agent.stdout.take().unwrap();
        write_frame(&mut stdin, &hello("probe", PROTO_VERSION)).unwrap();
        let _: v1::Event = read_frame(&mut stdout).unwrap().expect("hello ack");
        let probed = send_copy(
            &mut stdin,
            &mut stdout,
            4,
            &dropped_a,
            "stable",
            "dropped-a-key",
        );
        assert_eq!((probed.status.as_str(), probed.changed), ("ok", false));
        finish(&mut agent, &mut stdin, false);
    } else {
        assert_eq!(
            ledger_records(&dir),
            1,
            "a complete plan expires the tasks it dropped"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn filtered_done_prunes_nothing_and_a_complete_done_expires() {
    filtered_done_case(true);
    filtered_done_case(false);
}
