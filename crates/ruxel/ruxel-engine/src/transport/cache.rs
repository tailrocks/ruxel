use super::RemoteHost;
use anyhow::{Context, Result, ensure};
use ruxel_proto::constants::{AGENT_DIR, STATE_ROOT};

#[derive(Debug)]
struct Metadata {
    uid: u32,
    mode: u32,
    links: u64,
}

async fn metadata(remote: &RemoteHost, path: &str) -> Result<Option<Metadata>> {
    let output = remote
        .exec_output(&["stat", "-c", "%u:%f:%h", "--", path])
        .await?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = std::str::from_utf8(&output.stdout)?.trim();
    let fields: Vec<_> = text.split(':').collect();
    ensure!(fields.len() == 3, "invalid remote cache metadata");
    Ok(Some(Metadata {
        uid: fields[0].parse().context("cache owner UID")?,
        mode: u32::from_str_radix(fields[1], 16).context("cache mode")?,
        links: fields[2].parse().context("cache link count")?,
    }))
}

async fn directory(remote: &RemoteHost, path: &str, uid: u32, allow_root: bool) -> Result<()> {
    let metadata = metadata(remote, path)
        .await?
        .context("cannot inspect agent cache directory")?;
    ensure!(
        metadata.mode & 0o170_000 == 0o040_000,
        "agent cache ancestor must be a directory, not a symlink"
    );
    ensure!(
        metadata.uid == uid || (allow_root && metadata.uid == 0),
        "agent cache directory has an untrusted owner"
    );
    ensure!(
        metadata.mode & 0o022 == 0,
        "agent cache directory must not be group or other writable"
    );
    Ok(())
}

pub(super) async fn prepare(remote: &RemoteHost) -> Result<u32> {
    let identity = remote.exec_output(&["id", "-u"]).await?;
    ensure!(identity.status.success(), "cannot inspect SSH identity");
    let uid = std::str::from_utf8(&identity.stdout)?
        .trim()
        .parse()
        .context("SSH identity UID")?;
    for ancestor in ["/", "/var", "/var/lib"] {
        directory(remote, ancestor, uid, true).await?;
    }
    for managed in [STATE_ROOT, AGENT_DIR] {
        if !remote
            .exec_status(&["test", "-e", managed])
            .await?
            .success()
        {
            ensure!(
                remote
                    .exec_status(&["mkdir", "-m", "700", managed])
                    .await?
                    .success(),
                "create private agent cache directory"
            );
        }
        directory(remote, managed, uid, false).await?;
    }
    for managed in [STATE_ROOT, AGENT_DIR] {
        ensure!(
            remote
                .exec_status(&["chmod", "700", managed])
                .await?
                .success(),
            "protect agent cache directory"
        );
    }
    Ok(uid)
}

pub(super) async fn trusted_file(remote: &RemoteHost, path: &str, uid: u32) -> Result<bool> {
    Ok(metadata(remote, path).await?.is_some_and(|metadata| {
        metadata.mode & 0o170_000 == 0o100_000
            && metadata.uid == uid
            && metadata.mode & 0o022 == 0
            && metadata.links == 1
    }))
}
