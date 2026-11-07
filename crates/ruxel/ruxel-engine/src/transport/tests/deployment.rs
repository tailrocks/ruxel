#[path = "deployment_fixture.rs"]
mod fixture;
use fixture::Fixture;
use std::os::unix::fs::{PermissionsExt, symlink};

#[tokio::test]
async fn guarded_upload_verifies_before_executable_launch_and_repairs_cache() -> anyhow::Result<()>
{
    tokio::time::timeout(std::time::Duration::from_secs(15), exercise()).await??;
    Ok(())
}
async fn exercise() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let (connection, facts) = fixture.connect().await?;
    assert!(connection.uploaded_agent);
    assert_eq!(facts.facts.hostname, "synthetic-agent");
    connection.shutdown().await?;
    let uploaded = fixture.verify_first_upload()?;
    let cache = fixture.root.join("remote/var/lib/ruxel/agent");
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o755))?;
    let (connection, _) = fixture.connect().await?;
    assert!(!connection.uploaded_agent);
    connection.shutdown().await?;
    assert_eq!(
        std::fs::metadata(&cache)?.permissions().mode() & 0o777,
        0o700
    );
    std::fs::write(&uploaded, b"corrupt cached bytes")?;
    let (connection, _) = fixture.connect().await?;
    assert!(connection.uploaded_agent);
    connection.shutdown().await?;
    std::fs::write(fixture.root.join("mismatch"), b"synthetic hash mismatch")?;
    assert!(
        fixture
            .connect()
            .await
            .unwrap_err()
            .to_string()
            .contains("content verification")
    );
    let calls = fixture.calls()?;
    assert_eq!(calls.iter().filter(|event| event[0] == "launch").count(), 3);
    let uploads: std::collections::BTreeSet<_> = calls
        .iter()
        .filter(|event| event[0] == "open")
        .map(|event| event[1].as_str().unwrap())
        .collect();
    assert_eq!(uploads.len(), 3);
    assert!(uploads.iter().all(|path| path.contains(".tmp-")));
    let unrelated = fixture.root.join("unrelated");
    std::fs::create_dir(&unrelated)?;
    std::fs::set_permissions(&unrelated, std::fs::Permissions::from_mode(0o755))?;
    std::fs::remove_dir_all(&cache)?;
    symlink(&unrelated, &cache)?;
    assert!(
        fixture
            .connect()
            .await
            .unwrap_err()
            .to_string()
            .contains("symlink")
    );
    assert_eq!(
        std::fs::metadata(&unrelated)?.permissions().mode() & 0o777,
        0o755
    );
    assert!(std::fs::read_dir(&unrelated)?.next().is_none());
    std::fs::remove_file(&cache)?;
    std::fs::remove_dir(cache.parent().unwrap())?;
    symlink(&unrelated, cache.parent().unwrap())?;
    assert!(
        fixture
            .connect()
            .await
            .unwrap_err()
            .to_string()
            .contains("symlink")
    );
    assert_eq!(
        std::fs::metadata(&unrelated)?.permissions().mode() & 0o777,
        0o755
    );
    assert!(std::fs::read_dir(unrelated)?.next().is_none());
    Ok(())
}

#[tokio::test]
async fn failed_upload_and_cleanup_are_both_reported_without_launch() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    std::fs::write(fixture.root.join("sftp-fail"), b"synthetic write refusal")?;
    std::fs::write(
        fixture.root.join("cleanup-fail"),
        b"synthetic cleanup refusal",
    )?;
    let error = tokio::time::timeout(std::time::Duration::from_secs(15), fixture.connect())
        .await?
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("cleanup failed") && message.contains("23"));
    assert!(message.contains("sftp write agent"));
    assert!(!fixture.calls()?.iter().any(|event| event[0] == "launch"));
    Ok(())
}

#[path = "deployment_trust.rs"]
mod trust;

#[tokio::test]
async fn controller_refuses_agent_with_previous_protocol_version() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let ack = ruxel_proto::v1::Event {
        msg: Some(ruxel_proto::v1::event::Msg::HelloAck(
            ruxel_proto::v1::HelloAck {
                proto_version: 1,
                ..Default::default()
            },
        )),
    };
    std::fs::write(
        fixture.root.join("ack"),
        ruxel_proto::frame::encode_frame(&ack),
    )?;
    let error = tokio::time::timeout(std::time::Duration::from_secs(15), fixture.connect())
        .await?
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("agent speaks proto v1, controller v3")
    );
    Ok(())
}

#[path = "deployment_sensitivity.rs"]
mod sensitivity;
