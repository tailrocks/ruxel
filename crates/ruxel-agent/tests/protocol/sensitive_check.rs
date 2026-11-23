use super::*;

#[test]
fn sensitive_check_read_marks_state_before_restart() {
    let dir = temp_dir("sensitive-check");
    let target = dir.join("synthetic-input");
    std::fs::write(&target, "fixture").unwrap();
    let mut agent = spawn_agent(&dir);
    let mut stdin = agent.stdin.take().unwrap();
    let mut stdout = agent.stdout.take().unwrap();
    let mut greeting = hello("check", PROTO_VERSION);
    if let Some(Msg::Hello(value)) = &mut greeting.msg {
        value.check_mode = true;
    }
    write_frame(&mut stdin, &greeting).unwrap();
    let _: v1::Event = read_frame(&mut stdout).unwrap().unwrap();
    let mut plan = plan_with_params(
        1,
        "stat",
        serde_json::to_vec(&serde_json::json!({"path":target})).unwrap(),
        "",
    );
    if let Some(Msg::Plan(value)) = &mut plan.msg {
        value.tasks[0].no_log = true;
    }
    write_frame(&mut stdin, &plan).unwrap();
    let result = read_task_result(&mut stdout);
    assert_eq!(result.status, "ok");
    finish_agent(&mut agent, &mut stdin);
    assert_eq!(std::fs::read(dir.join("sensitive-effects")).unwrap(), b"");

    let mut restarted = spawn_agent(&dir);
    let mut stdin = restarted.stdin.take().unwrap();
    let mut stdout = restarted.stdout.take().unwrap();
    write_frame(&mut stdin, &hello("restart", PROTO_VERSION)).unwrap();
    let event: v1::Event = read_frame(&mut stdout).unwrap().unwrap();
    let Some(v1::event::Msg::HelloAck(ack)) = event.msg else {
        panic!("expected hello acknowledgement")
    };
    assert_eq!(ack.sensitive_effects, Some(true));
    finish_agent(&mut restarted, &mut stdin);
    std::fs::remove_dir_all(dir).unwrap();
}
