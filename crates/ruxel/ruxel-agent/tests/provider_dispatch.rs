use ruxel_proto::{
    PROTO_VERSION,
    frame::{read_frame, write_frame},
    v1,
};
use std::process::{Command, Stdio};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const CASES: &[(&str, &str, bool)] = &[
    ("shell", "printf synthetic-shell", true),
    ("unknown-operation", "", false),
    ("ansible.posix.sysctl", "", false),
    ("volume_group", "", true),
    ("logical_volume", "", true),
    ("timezone", "", true),
    ("mount", "", true),
    ("postgres_database", "", true),
    ("postgres_role", "", true),
    ("postgres_schema", "", true),
    ("postgres_privileges", "", true),
    ("community.general.lvg", "", false),
    ("community.general.lvol", "", false),
    ("community.general.timezone", "", false),
    ("ansible.posix.mount", "", false),
    ("community.postgresql.postgresql_db", "", false),
    ("community.postgresql.postgresql_user", "", false),
    ("community.postgresql.postgresql_schema", "", false),
    ("community.postgresql.postgresql_privs", "", false),
];

#[test]
fn migrated_shell_is_reachable_and_unknown_modules_fail_cleanly() -> Result<()> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let state = std::env::temp_dir().join(format!("ruxel-dispatch-{}-{nonce}", std::process::id()));
    let mut agent = Command::new(env!("CARGO_BIN_EXE_ruxel-agent"))
        .env("RUXEL_STATE_DIR", &state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = agent.stdin.take().ok_or("missing stdin")?;
    let mut output = agent.stdout.take().ok_or("missing stdout")?;
    write_frame(
        &mut input,
        &v1::Envelope {
            msg: Some(v1::envelope::Msg::Hello(v1::Hello {
                proto_version: PROTO_VERSION,
                no_cache: true,
                ..Default::default()
            })),
        },
    )?;
    let hello: v1::Event = read_frame(&mut output)?.ok_or("missing acknowledgement")?;
    assert!(matches!(hello.msg, Some(v1::event::Msg::HelloAck(_))));
    for (index, &(module, command, supported)) in CASES.iter().enumerate() {
        let id = u64::try_from(index)? + 1;
        write_frame(
            &mut input,
            &v1::Envelope {
                msg: Some(v1::envelope::Msg::Plan(v1::Plan {
                    tasks: vec![v1::RenderedTask {
                        task_id: id,
                        module: module.into(),
                        rendered: true,
                        iterations: vec![v1::Iteration {
                            params_json: b"{}".to_vec(),
                            free_form: command.into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
            },
        )?;
        loop {
            let event: v1::Event = read_frame(&mut output)?.ok_or("agent ended before result")?;
            if let Some(v1::event::Msg::TaskResult(result)) = event.msg {
                assert_eq!(result.task_id, id);
                let value: serde_json::Value = serde_json::from_slice(&result.result_json)?;
                if module == "shell" {
                    assert_eq!(result.status, "changed");
                    assert_eq!(value["stdout"], "synthetic-shell");
                } else {
                    assert_eq!(result.status, "failed");
                    let message = value["msg"].as_str().ok_or("missing error message")?;
                    assert_eq!(
                        !message.contains("not implemented"),
                        supported,
                        "{module}: {message}"
                    );
                }
                break;
            }
        }
    }
    write_frame(
        &mut input,
        &v1::Envelope {
            msg: Some(v1::envelope::Msg::Done(v1::Done {})),
        },
    )?;
    assert!(agent.wait()?.success());
    std::fs::remove_dir_all(state)?;
    Ok(())
}
