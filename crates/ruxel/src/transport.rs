//! Controller-side transport (ARCHITECTURE §2): one SSH connection per
//! host for the whole run — the system OpenSSH via ControlMaster
//! native-mux, so the operator's ~/.ssh/config, keys, and known_hosts
//! behave exactly like their ansible-playbook runs. The agent binary is
//! uploaded content-addressed (blake3) over a muxed SFTP channel only
//! when the hash is not already present; the protocol then runs framed
//! over the spawned agent's stdio.

use anyhow::{Context, Result, bail};
use openssh::{KnownHosts, Session, Stdio};
use prost::Message;
use ruxel_proto::PROTO_VERSION;
use ruxel_proto::v1::{self, envelope::Msg as EnvMsg, event::Msg as EvMsg};
use std::path::Path;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const AGENT_DIR: &str = "/var/lib/ruxel/agent";

/// One connected host: the muxed SSH session and the running agent.
pub struct AgentConnection {
    /// Kept alive for the run; the agent child borrows the master via mux.
    _session: std::sync::Arc<Session>,
    child: openssh::Child<std::sync::Arc<Session>>,
    stdin: openssh::ChildStdin,
    stdout: openssh::ChildStdout,
    /// True when this run had to upload the agent (hash was missing).
    pub uploaded_agent: bool,
}

pub struct HostFacts {
    pub facts: v1::Facts,
    pub agent_version: String,
    pub ledger_generation: u64,
}

/// Connect, ensure the agent binary, spawn it, and complete the handshake.
pub async fn connect(
    destination: &str,
    agent_binary: &Path,
    run_id: &str,
) -> Result<(AgentConnection, HostFacts)> {
    let session = std::sync::Arc::new(
        Session::connect_mux(destination, KnownHosts::Strict)
            .await
            .with_context(|| format!("ssh connect to {destination}"))?,
    );

    let agent_bytes = std::fs::read(agent_binary)
        .with_context(|| format!("read agent binary {}", agent_binary.display()))?;
    let b3 = blake3::hash(&agent_bytes).to_hex().to_string();
    let remote_path = format!("{AGENT_DIR}/{b3}");

    let uploaded_agent = ensure_agent(&session, &remote_path, &agent_bytes).await?;

    let mut child = session
        .clone()
        .arc_command(&remote_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .await
        .context("spawn agent")?;
    let mut stdin = child.stdin().take().context("agent stdin")?;
    let mut stdout = child.stdout().take().context("agent stdout")?;

    write_frame(
        &mut stdin,
        &v1::Envelope {
            msg: Some(EnvMsg::Hello(v1::Hello {
                proto_version: PROTO_VERSION,
                run_id: run_id.to_string(),
                ..Default::default()
            })),
        },
    )
    .await?;

    let event: v1::Event = match read_frame(&mut stdout).await? {
        Some(ev) => ev,
        None => bail!("agent closed the stream before HelloAck"),
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
            _session: session,
            child,
            stdin,
            stdout,
            uploaded_agent,
        },
        HostFacts {
            facts: ack.facts.unwrap_or_default(),
            agent_version: ack.agent_version,
            ledger_generation: ack.ledger_generation,
        },
    ))
}

impl AgentConnection {
    /// The agent's event stream (everything after HelloAck) — consumed by
    /// the scheduler from M3 on.
    pub fn event_stream(&mut self) -> &mut openssh::ChildStdout {
        &mut self.stdout
    }

    /// Send Done and wait for the agent to flush and exit cleanly.
    pub async fn shutdown(mut self) -> Result<()> {
        write_frame(
            &mut self.stdin,
            &v1::Envelope {
                msg: Some(EnvMsg::Done(v1::Done {})),
            },
        )
        .await?;
        let status = self.child.wait().await.context("agent exit")?;
        if !status.success() {
            bail!("agent exited with {status:?}");
        }
        Ok(())
    }
}

/// Content-addressed agent provisioning: if `remote_path` is already an
/// executable file, nothing moves; otherwise upload via a muxed SFTP
/// channel and mark executable. Returns whether an upload happened.
async fn ensure_agent(session: &Session, remote_path: &str, bytes: &[u8]) -> Result<bool> {
    let present = session
        .command("test")
        .arg("-x")
        .arg(remote_path)
        .status()
        .await
        .context("test -x agent")?
        .success();
    if present {
        return Ok(false); // already provisioned at this hash
    }

    session
        .command("mkdir")
        .arg("-p")
        .arg(AGENT_DIR)
        .status()
        .await
        .context("mkdir agent dir")?;

    // A second muxed client over the same master carries the SFTP channel
    // (ARCHITECTURE: ch1), leaving ch0 free for the protocol stream.
    let mut sftp_child = session
        .subsystem("sftp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .await
        .context("open sftp subsystem")?;
    let sftp = openssh_sftp_client::Sftp::new(
        sftp_child.stdin().take().context("sftp stdin")?,
        sftp_child.stdout().take().context("sftp stdout")?,
        openssh_sftp_client::SftpOptions::new(),
    )
    .await
    .context("sftp handshake")?;

    let tmp_path = format!("{remote_path}.tmp-{}", std::process::id());
    {
        let mut file = sftp
            .options()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .await
            .context("sftp create tmp")?;
        file.write_all(bytes).await.context("sftp write agent")?;
        file.close().await.context("sftp close")?;
    }
    sftp.fs()
        .rename(&tmp_path, remote_path)
        .await
        .context("rename agent into place")?;
    sftp.close().await.context("sftp shutdown")?;

    session
        .command("chmod")
        .arg("755")
        .arg(remote_path)
        .status()
        .await
        .context("chmod agent")?;

    Ok(true)
}

// -- Async framing (same wire format as ruxel_proto::frame) -----------------

async fn write_frame<M: Message>(w: &mut (impl AsyncWrite + Unpin), msg: &M) -> Result<()> {
    let mut buf = Vec::with_capacity(msg.encoded_len() + 5);
    msg.encode_length_delimited(&mut buf)
        .expect("Vec<u8> write is infallible");
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

async fn read_frame<M: Message + Default>(r: &mut (impl AsyncRead + Unpin)) -> Result<Option<M>> {
    let mut len: u64 = 0;
    let mut shift = 0u32;
    let mut first = true;
    loop {
        let mut byte = [0u8; 1];
        let n = r.read(&mut byte).await?;
        if n == 0 {
            if first {
                return Ok(None);
            }
            bail!("EOF inside frame length");
        }
        first = false;
        len |= u64::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            bail!("frame length varint overflow");
        }
    }
    if len > ruxel_proto::frame::MAX_FRAME_LEN {
        bail!("frame of {len} bytes exceeds MAX_FRAME_LEN");
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    Ok(Some(M::decode(body.as_slice())?))
}
