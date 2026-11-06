mod local;
pub use local::LocalConnection;

use anyhow::{Context, Result, bail};
use prost::Message;
use ruxel_proto::PROTO_VERSION;
use ruxel_proto::constants::{AGENT_DIR, HANDSHAKE_TIMEOUT_SECS};
use ruxel_proto::v1::{self, envelope::Msg as EnvMsg, event::Msg as EvMsg};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

#[derive(Debug, Default, Clone)]
pub struct ConnectOptions {
    pub port: Option<u16>,
    pub keyfile: Option<PathBuf>,
    pub accept_new_host_key: bool,
    pub known_hosts_file: Option<PathBuf>,
    pub diff_mode: bool,
    pub no_cache: bool,
}

struct RemoteHost {
    destination: String,
    options: ConnectOptions,
}

impl RemoteHost {
    fn client(&self) -> Command {
        let mut command = Command::new("ssh");
        command.args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=15",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=4",
        ]);
        apply_options(&mut command, &self.options);
        command.arg(&self.destination);
        command
    }
    async fn exec_status(&self, argv: &[&str]) -> Result<std::process::ExitStatus> {
        let mut cmd = self.client();
        cmd.arg("--");
        for a in argv {
            cmd.arg(a);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.status().await.context("ssh exec")
    }

    async fn exec_output(&self, argv: &[&str]) -> Result<std::process::Output> {
        let mut cmd = self.client();
        cmd.arg("--");
        for arg in argv {
            cmd.arg(arg);
        }
        cmd.stdin(Stdio::null()).stderr(Stdio::piped());
        cmd.output().await.context("ssh exec output")
    }
}

fn apply_options(cmd: &mut Command, options: &ConnectOptions) {
    if let Some(port) = options.port {
        cmd.arg("-p").arg(port.to_string());
    }
    if let Some(keyfile) = &options.keyfile {
        cmd.arg("-i")
            .arg(keyfile)
            .arg("-o")
            .arg("IdentitiesOnly=yes");
    }
    if let Some(kh) = &options.known_hosts_file {
        cmd.arg("-o")
            .arg(format!("UserKnownHostsFile={}", kh.display()));
    }
    cmd.arg("-o").arg(if options.accept_new_host_key {
        "StrictHostKeyChecking=accept-new"
    } else {
        "StrictHostKeyChecking=yes"
    });
}

#[derive(Debug)]
pub struct AgentConnection {
    agent: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    pub uploaded_agent: bool,
}

#[derive(Debug)]
pub struct HostFacts {
    pub facts: v1::Facts,
}

/// Connect and verify an agent over the PATH-selected SSH client.
///
/// # Errors
/// Returns an error when agent upload, verification, or protocol negotiation fails.
pub async fn connect_with(
    destination: &str,
    agent_binary: &Path,
    run_id: &str,
    check_mode: bool,
    options: &ConnectOptions,
) -> Result<(AgentConnection, HostFacts)> {
    let remote = RemoteHost {
        destination: destination.to_owned(),
        options: options.clone(),
    };

    let agent_bytes = std::fs::read(agent_binary)
        .with_context(|| format!("read agent binary {}", agent_binary.display()))?;
    let b3 = blake3::hash(&agent_bytes).to_hex().to_string();
    let remote_path = format!("{AGENT_DIR}/{b3}");

    let uploaded_agent = ensure_agent(&remote, &remote_path, &agent_bytes).await?;

    let mut agent_cmd = remote.client();
    agent_cmd
        .arg("--")
        .arg(&remote_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut agent = agent_cmd.spawn().context("spawn agent")?;
    let mut stdin = agent.stdin.take().context("agent stdin")?;
    let mut stdout = agent.stdout.take().context("agent stdout")?;

    write_frame(
        &mut stdin,
        &v1::Envelope {
            msg: Some(EnvMsg::Hello(v1::Hello {
                proto_version: PROTO_VERSION,
                run_id: run_id.to_string(),
                check_mode,
                diff_mode: options.diff_mode,
                no_cache: options.no_cache,
            })),
        },
    )
    .await?;

    let ack_read = read_frame::<v1::Event>(&mut stdout);
    let event = match tokio::time::timeout(
        std::time::Duration::from_secs(HANDSHAKE_TIMEOUT_SECS),
        ack_read,
    )
    .await
    {
        Ok(result) => match result? {
            Some(ev) => ev,
            None => bail!("agent closed the stream before HelloAck"),
        },
        Err(_) => bail!("timed out waiting for HelloAck from {destination}"),
    };
    let ack = match event.msg {
        Some(EvMsg::HelloAck(ack)) => ack,
        Some(EvMsg::Log(log)) => bail!("agent refused handshake: {}", log.message),
        other => bail!("expected HelloAck, got {other:?}"),
    };
    if ack.proto_version != PROTO_VERSION {
        bail!(
            "agent speaks proto v{}, controller v{PROTO_VERSION}",
            ack.proto_version
        );
    }

    Ok((
        AgentConnection {
            agent,
            stdin,
            stdout,
            uploaded_agent,
        },
        HostFacts {
            facts: ack.facts.unwrap_or_default(),
        },
    ))
}

impl AgentConnection {
    /// Send a framed request.
    ///
    /// # Errors
    /// Returns protocol encoding or stream write errors.
    pub async fn send(&mut self, envelope: &v1::Envelope) -> Result<()> {
        write_frame(&mut self.stdin, envelope).await
    }

    /// Read one framed event.
    ///
    /// # Errors
    /// Returns malformed frame or stream read errors.
    pub async fn next_event(&mut self) -> Result<Option<v1::Event>> {
        read_frame(&mut self.stdout).await
    }

    /// Flush the ledger and wait for agent termination.
    ///
    /// # Errors
    /// Returns stream errors or an unsuccessful agent exit.
    pub async fn shutdown(mut self) -> Result<()> {
        write_frame(
            &mut self.stdin,
            &v1::Envelope {
                msg: Some(EnvMsg::Done(v1::Done {})),
            },
        )
        .await?;
        let status = self.agent.wait().await.context("agent exit")?;
        if !status.success() {
            bail!("agent exited with {status:?}");
        }
        Ok(())
    }
}

static UPLOAD_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

async fn ensure_agent(remote: &RemoteHost, remote_path: &str, bytes: &[u8]) -> Result<bool> {
    let exists = remote
        .exec_status(&["test", "-x", remote_path])
        .await?
        .success();
    if exists && remote_agent_hash_matches(remote, remote_path, bytes).await? {
        return Ok(false);
    }

    remote
        .exec_status(&["mkdir", "-p", AGENT_DIR])
        .await?
        .success()
        .then_some(())
        .context("mkdir agent dir")?;

    let mut sftp_cmd = remote.client();
    sftp_cmd
        .arg("-s")
        .arg("sftp")
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
        if let Err(cleanup) = remote.exec_status(&["rm", "-f", &tmp_path]).await {
            return Err(error.context(format!("upload cleanup also failed: {cleanup}")));
        }
        return Err(error);
    }

    remote
        .exec_status(&["chmod", "755", remote_path])
        .await?
        .success()
        .then_some(())
        .context("chmod agent")?;

    if !remote_agent_hash_matches(remote, remote_path, bytes).await? {
        bail!("uploaded agent failed remote content verification");
    }

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

async fn write_frame<M: Message>(w: &mut (impl AsyncWrite + Unpin), msg: &M) -> Result<()> {
    let buf = ruxel_proto::frame::encode_frame(msg);
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

async fn read_frame<M: Message + Default>(r: &mut (impl AsyncRead + Unpin)) -> Result<Option<M>> {
    let mut decoder = ruxel_proto::frame::VarintDecoder::default();
    let mut first = true;
    let len = loop {
        let mut byte = [0u8; 1];
        let n = match r.read(&mut byte).await {
            Ok(n) => n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        if n == 0 {
            if first {
                return Ok(None);
            }
            bail!("EOF inside frame length");
        }
        first = false;
        match decoder.push(byte[0]) {
            Ok(Some(len)) => break len,
            Ok(None) => {}
            Err(message) => bail!(message),
        }
    };
    if len > ruxel_proto::frame::MAX_FRAME_LEN {
        bail!("frame of {len} bytes exceeds MAX_FRAME_LEN");
    }
    let mut body = vec![0u8; usize::try_from(len).context("frame length exceeds address space")?];
    r.read_exact(&mut body).await?;
    Ok(Some(M::decode(body.as_slice())?))
}

#[cfg(test)]
mod tests;
