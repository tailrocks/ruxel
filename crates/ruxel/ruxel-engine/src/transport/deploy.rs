use super::RemoteHost;
use anyhow::{Context, Result, bail};
use ruxel_proto::constants::{AGENT_DIR, STATE_ROOT};
use sha2::{Digest, Sha256};
use std::process::Stdio;

static UPLOAD_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(super) async fn ensure_agent(
    remote: &RemoteHost,
    remote_path: &str,
    bytes: &[u8],
) -> Result<bool> {
    prepare_cache(remote).await?;
    let exists = remote
        .exec_status(&["test", "-x", remote_path])
        .await?
        .success();
    if exists && remote_agent_hash_matches(remote, remote_path, bytes).await? {
        return Ok(false);
    }

    let mut sftp_cmd = remote.subsystem();
    sftp_cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut sftp_child = sftp_cmd.spawn().context("spawn sftp channel")?;
    let sftp = openssh_sftp_client::Sftp::new(
        sftp_child.stdin.take().context("sftp stdin")?,
        sftp_child.stdout.take().context("sftp stdout")?,
        openssh_sftp_client::SftpOptions::new(),
    )
    .await
    .context("sftp handshake")?;

    let sequence = UPLOAD_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let tmp_path = format!(
        "{remote_path}.tmp-{}-{timestamp}-{sequence}",
        std::process::id()
    );
    let upload_result: Result<()> = async {
        let mut file = sftp
            .options()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .await
            .context("sftp create tmp")?;
        file.write_all(bytes).await.context("sftp write agent")?;
        file.close().await.context("sftp close")?;
        sftp.fs()
            .rename(&tmp_path, remote_path)
            .await
            .context("rename agent into place")?;
        sftp.close().await.context("sftp shutdown")?;
        Ok(())
    }
    .await;
    if let Err(error) = upload_result {
        match remote.exec_status(&["rm", "-f", &tmp_path]).await {
            Ok(status) if status.success() => {}
            Ok(status) => return Err(error.context(format!("upload cleanup failed: {status}"))),
            Err(cleanup) => {
                return Err(error.context(format!("upload cleanup also failed: {cleanup}")));
            }
        }
        return Err(error);
    }

    if !remote_agent_hash_matches(remote, remote_path, bytes).await? {
        bail!("uploaded agent failed remote content verification");
    }
    remote
        .exec_status(&["chmod", "755", remote_path])
        .await?
        .success()
        .then_some(())
        .context("chmod agent")?;

    Ok(true)
}

const HEX: &[u8; 16] = b"0123456789abcdef";

async fn remote_agent_hash_matches(
    remote: &RemoteHost,
    remote_path: &str,
    bytes: &[u8],
) -> Result<bool> {
    let output = remote.exec_output(&["sha256sum", remote_path]).await?;
    if !output.status.success() {
        return Ok(false);
    }
    let remote = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let digest = Sha256::digest(bytes);
    let mut expected = String::with_capacity(digest.len() * 2);

    for byte in digest {
        expected.push(HEX[(byte >> 4) as usize] as char);
        expected.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(remote == expected)
}

async fn prepare_cache(remote: &RemoteHost) -> Result<()> {
    for directory in [STATE_ROOT, AGENT_DIR] {
        remote
            .exec_status(&["test", "!", "-L", directory])
            .await?
            .success()
            .then_some(())
            .context("agent cache must not be a symlink")?;
    }
    remote
        .exec_status(&["mkdir", "-p", AGENT_DIR])
        .await?
        .success()
        .then_some(())
        .context("mkdir agent dir")?;

    remote
        .exec_status(&["chmod", "700", AGENT_DIR])
        .await?
        .success()
        .then_some(())
        .context("protect agent cache directory")?;

    Ok(())
}
