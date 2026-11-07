use super::Fixture;
use serde_json::json;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

#[tokio::test]
async fn untrusted_directory_chain_is_refused_before_permission_changes() -> anyhow::Result<()> {
    for path in [
        "/",
        "/var",
        "/var/lib",
        "/var/lib/ruxel",
        "/var/lib/ruxel/agent",
    ] {
        for foreign_owner in [false, true] {
            let fixture = Fixture::new()?;
            std::fs::create_dir_all(fixture.root.join("remote/var/lib/ruxel/agent"))?;
            for managed in ["remote/var/lib/ruxel", "remote/var/lib/ruxel/agent"] {
                std::fs::set_permissions(
                    fixture.root.join(managed),
                    std::fs::Permissions::from_mode(0o755),
                )?;
            }
            let local = fixture
                .root
                .join("remote")
                .join(path.trim_start_matches('/'));
            let mode = if foreign_owner { 0o755 } else { 0o777 };
            std::fs::set_permissions(&local, std::fs::Permissions::from_mode(mode))?;
            if foreign_owner {
                let owner = std::fs::metadata(&local)?.uid() + 1;
                fixture.override_owner(path, owner, None)?;
            }
            let error = fixture.connect().await.unwrap_err().to_string();
            assert!(
                error.contains(if foreign_owner {
                    "untrusted owner"
                } else {
                    "writable"
                }),
                "{error}"
            );
            assert_eq!(
                std::fs::metadata(&local)?.permissions().mode() & 0o777,
                mode
            );
            for managed in ["/var/lib/ruxel", "/var/lib/ruxel/agent"] {
                let directory = fixture
                    .root
                    .join("remote")
                    .join(managed.trim_start_matches('/'));
                assert_eq!(
                    std::fs::metadata(directory)?.permissions().mode() & 0o777,
                    if managed == path { mode } else { 0o755 }
                );
            }
            for call in fixture.calls()? {
                if call[0] == "ssh" {
                    let args = call[1].as_array().unwrap();
                    assert!(
                        !(args.iter().any(|arg| arg == "chmod")
                            && args.last() == Some(&json!(path)))
                    );
                }
                assert_ne!(call[0], "launch");
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn unsafe_cached_objects_are_replaced_without_following_or_mutating_them()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let (connection, _) = fixture.connect().await?;
    connection.shutdown().await?;
    let cached = fixture.verify_first_upload()?;
    std::fs::set_permissions(&cached, std::fs::Permissions::from_mode(0o666))?;
    let before = std::fs::metadata(&cached)?.ino();
    let (connection, _) = fixture.connect().await?;
    assert!(connection.uploaded_agent);
    connection.shutdown().await?;
    assert_ne!(std::fs::metadata(&cached)?.ino(), before);
    let relative = cached.strip_prefix(fixture.root.join("remote"))?;
    let remote_path = format!("/{}", relative.display());
    let old = std::fs::metadata(&cached)?;
    fixture.override_owner(&remote_path, old.uid() + 1, Some(old.ino()))?;
    let (connection, _) = fixture.connect().await?;
    assert!(connection.uploaded_agent);
    connection.shutdown().await?;
    assert_ne!(std::fs::metadata(&cached)?.ino(), old.ino());
    std::fs::remove_file(fixture.root.join("metadata"))?;
    let victim = fixture.root.join("unrelated executable");
    std::fs::copy(fixture.root.join("agent"), &victim)?;
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o755))?;
    let victim_inode = std::fs::metadata(&victim)?.ino();
    for hard_link in [false, true] {
        std::fs::remove_file(&cached)?;
        if hard_link {
            std::fs::hard_link(&victim, &cached)?;
        } else {
            symlink(&victim, &cached)?;
        }
        let (connection, _) = fixture.connect().await?;
        assert!(connection.uploaded_agent);
        connection.shutdown().await?;
        assert_eq!(std::fs::metadata(&victim)?.ino(), victim_inode);
        assert_eq!(
            std::fs::metadata(&victim)?.permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            std::fs::read(&victim)?,
            std::fs::read(fixture.root.join("agent"))?
        );
    }
    Ok(())
}
