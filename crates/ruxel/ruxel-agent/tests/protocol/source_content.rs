use super::{finish_agent, hello, read_task_result, spawn_agent, temp_dir};
use ruxel_proto::{
    PROTO_VERSION,
    frame::{read_frame, write_frame},
    v1,
};

#[derive(Debug)]
struct Agent(std::process::Child);

impl Drop for Agent {
    fn drop(&mut self) {
        drop(self.0.kill());
        drop(self.0.wait());
    }
}

#[test]
fn binary_source_empty_files_and_invalid_payloads_use_actual_agent()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = temp_dir("binary-source");
    let dest = directory.join("payload");
    let invalid_dest = directory.join("directory");
    std::fs::create_dir(&invalid_dest)?;
    let mut agent = Agent(spawn_agent(&directory.join("state")));
    let mut input = agent.0.stdin.take().ok_or("stdin")?;
    let mut output = agent.0.stdout.take().ok_or("stdout")?;
    write_frame(&mut input, &hello("binary-source", PROTO_VERSION))?;
    let _: v1::Event = read_frame(&mut output)?.ok_or("hello ack")?;
    let binary = vec![0xff, 0x80, 0, 1];
    for (index, (module, params, source_content, key, failed)) in
        cases(&dest, &invalid_dest, &binary).into_iter().enumerate()
    {
        write_frame(
            &mut input,
            &v1::Envelope {
                msg: Some(v1::envelope::Msg::Plan(v1::Plan {
                    tasks: vec![v1::RenderedTask {
                        task_id: u64::try_from(index)?,
                        module: module.into(),
                        rendered: true,
                        iterations: vec![v1::Iteration {
                            params_json: serde_json::to_vec(&params)?,
                            source_content,
                            ledger_key: key.into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
            },
        )?;
        let result = read_task_result(&mut output);
        assert_eq!(result.status == "failed", failed);
        let expected = if index >= 4 {
            &[][..]
        } else {
            binary.as_slice()
        };
        assert_eq!(std::fs::read(&dest)?, expected);
    }
    assert!(invalid_dest.is_dir());
    finish_agent(&mut agent.0, &mut input);
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

type Case = (
    &'static str,
    serde_json::Value,
    Option<Vec<u8>>,
    &'static str,
    bool,
);

fn cases(dest: &std::path::Path, invalid_dest: &std::path::Path, binary: &[u8]) -> [Case; 7] {
    [
        (
            "copy",
            serde_json::json!({"dest":dest}),
            Some(binary.to_vec()),
            "binary",
            false,
        ),
        (
            "copy",
            serde_json::json!({"dest":dest,"content":"conflict"}),
            Some(binary.to_vec()),
            "binary",
            true,
        ),
        (
            "file",
            serde_json::json!({"path":dest,"state":"absent"}),
            Some(binary.to_vec()),
            "binary",
            true,
        ),
        (
            "copy",
            serde_json::json!({"dest":dest}),
            None,
            "absent",
            true,
        ),
        (
            "copy",
            serde_json::json!({"dest":dest}),
            Some(Vec::new()),
            "empty",
            false,
        ),
        (
            "copy",
            serde_json::json!({"dest":invalid_dest}),
            Some(Vec::new()),
            "invalid-dest",
            true,
        ),
        (
            "copy",
            serde_json::json!({"dest":dest,"force":false}),
            Some(binary.to_vec()),
            "no-force",
            false,
        ),
    ]
}
