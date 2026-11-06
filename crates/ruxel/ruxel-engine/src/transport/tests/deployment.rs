#[path = "deployment_fixture.rs"]
mod fixture;
use fixture::Fixture;
use prost::Message;
use ruxel_proto::v1::{self, envelope::Msg};
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
    let uploaded = verify_first_upload(&fixture)?;
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

fn verify_first_upload(fixture: &Fixture) -> anyhow::Result<std::path::PathBuf> {
    let calls = fixture.calls()?;
    let remote = calls.iter().find(|event| event[0] == "launch").unwrap()[1]
        .as_str()
        .unwrap();
    let cache = fixture.root.join("remote/var/lib/ruxel/agent");
    let uploaded = fixture
        .root
        .join("remote")
        .join(remote.trim_start_matches('/'));
    assert_eq!(
        std::fs::read(&uploaded)?,
        std::fs::read(fixture.root.join("agent"))?
    );
    assert_eq!(
        std::fs::metadata(&cache)?.permissions().mode() & 0o777,
        0o700
    );
    let position = |command: &str| {
        calls
            .iter()
            .position(|event| {
                event[0] == "ssh"
                    && event[1]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|arg| arg == command)
            })
            .unwrap()
    };
    assert!(position("sha256sum") < position("755"));
    assert!(position("755") < calls.iter().position(|event| event[0] == "launch").unwrap());
    let hello = v1::Envelope::decode(std::fs::read(fixture.root.join("hello"))?.as_slice())?;
    assert!(
        matches!(hello.msg, Some(Msg::Hello(h)) if h.run_id == "deployment-test" && h.check_mode && h.diff_mode && h.no_cache)
    );
    let done = v1::Envelope::decode(std::fs::read(fixture.root.join("done"))?.as_slice())?;
    assert!(matches!(done.msg, Some(Msg::Done(_))));
    Ok(uploaded)
}
