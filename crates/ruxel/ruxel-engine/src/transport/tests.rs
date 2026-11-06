#![expect(
    clippy::unwrap_used,
    reason = "tests require successful protocol frames"
)]
use super::{ConnectOptions, RemoteHost, read_frame, write_frame};
use ruxel_proto::v1;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn ssh_uses_guard_compatible_commands_with_strict_host_keys() {
    let host = RemoteHost {
        destination: "synthetic.invalid".into(),
        test_path: None,
        options: ConnectOptions::default(),
    };
    let command = host.client();
    let args: Vec<_> = command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(command.as_std().get_program(), "ssh");
    assert!(
        !args
            .iter()
            .any(|arg| ["-M", "-S", "-O", "-A", "-J"].contains(&arg.as_str()))
    );
    assert!(args.iter().any(|arg| arg == "StrictHostKeyChecking=yes"));
    assert_eq!(args.last().map(String::as_str), Some("synthetic.invalid"));
    assert_eq!(args[args.len() - 2], "--");
    let subsystem = host.subsystem();
    let args: Vec<_> = subsystem.as_std().get_args().collect();
    assert_eq!(
        &args[args.len() - 4..],
        ["-s", "--", "synthetic.invalid", "sftp"]
    );
}

#[tokio::test]
async fn async_and_sync_framing_are_byte_identical() {
    let message = v1::Envelope {
        msg: Some(v1::envelope::Msg::Hello(v1::Hello {
            proto_version: ruxel_proto::PROTO_VERSION,
            run_id: "codec-identity".into(),
            ..Default::default()
        })),
    };
    let sync = ruxel_proto::frame::encode_frame(&message);
    let (mut writer, mut reader) = tokio::io::duplex(1024);
    write_frame(&mut writer, &message).await.unwrap();
    let mut asynchronous = vec![0; sync.len()];
    reader.read_exact(&mut asynchronous).await.unwrap();
    assert_eq!(asynchronous, sync);

    let (mut writer, mut reader) = tokio::io::duplex(1024);
    writer.write_all(&sync).await.unwrap();
    let decoded: v1::Envelope = read_frame(&mut reader).await.unwrap().unwrap();
    assert_eq!(decoded, message);
}

#[cfg(unix)]
#[tokio::test]
async fn ssh_options_reach_only_an_injected_fake_executable() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("ruxel-fake-ssh-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let program = root.join("ssh");
    std::fs::write(&program, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n")?;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))?;
    let host = RemoteHost {
        destination: "fixture.invalid".into(),
        test_path: None,
        options: ConnectOptions {
            keyfile: Some(root.join("synthetic identity")),
            known_hosts_file: Some(root.join("synthetic known hosts")),
            accept_new_host_key: true,
            ..Default::default()
        },
    };
    let output = host.client().env("PATH", &root).output().await?;
    assert!(output.status.success());
    let args = String::from_utf8(output.stdout)?;
    assert!(args.contains("StrictHostKeyChecking=accept-new\n"));
    assert!(args.contains("synthetic identity\n"));
    assert!(args.contains("synthetic known hosts\n"));
    assert!(args.ends_with("fixture.invalid\n"));
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[path = "tests/deployment.rs"]
mod deployment;
