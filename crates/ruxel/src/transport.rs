//! Controller-side transport (ARCHITECTURE §2): one SSH ControlMaster
//! connection per host for the whole run, driven directly through the
//! system OpenSSH via tokio::process — the operator's ~/.ssh/config, keys,
//! and known_hosts behave exactly like their ansible-playbook runs, and
//! every channel (exec, sftp, the agent's stdio stream) is a plain
//! `ssh -S <socket>` mux client whose pipes we own end to end.
//!
//! Known issue (2026-06-11, fixture-reproduced, also present with the
//! openssh crate this replaced): the SECOND sequential connect inside one
//! process stalls at the agent handshake — first connect and concurrent
//! shell-driven repeats are fine. Real runs open one connection per host
//! per process today; revisit before M5's multi-host parallelism (which
//! needs concurrent, not sequential, sessions). Gate evidence therefore
//! runs each connect in its own process (tools/fixtures/gate.sh).

use anyhow::{Context, Result, bail};
use prost::Message;
use ruxel_proto::PROTO_VERSION;
use ruxel_proto::v1::{self, envelope::Msg as EnvMsg, event::Msg as EvMsg};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const AGENT_DIR: &str = "/var/lib/ruxel/agent";

/// Connection tuning beyond the operator's ssh config. Production runs use
/// `Default` (config-driven, strict known_hosts, exactly like ansible);
/// fixture/test runs point at an ephemeral keyfile and accept new hosts.
#[derive(Default, Clone)]
pub struct ConnectOptions {
    pub keyfile: Option<PathBuf>,
    pub accept_new_host_key: bool,
}

/// The per-host ControlMaster: a foreground `ssh -M -N` child owning the
/// TCP connection; every command/channel is a mux client on its socket.
struct Master {
    destination: String,
    socket: PathBuf,
    options: ConnectOptions,
    process: Child,
}

impl Master {
    async fn establish(destination: &str, options: &ConnectOptions) -> Result<Self> {
        let socket = std::env::temp_dir().join(format!(
            "ruxel-mux-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));
        let mut cmd = Command::new("ssh");
        cmd.arg("-M")
            .arg("-N")
            .arg("-S")
            .arg(&socket)
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=15");
        apply_options(&mut cmd, options);
        cmd.arg(destination)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut process = cmd.spawn().context("spawn ssh master")?;
        let mut stderr = process.stderr.take().expect("piped");

        // Wait for the control socket, surfacing ssh's own error if the
        // master dies first (auth failure, unreachable, host key).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            if socket.exists() {
                break;
            }
            if let Some(status) = process.try_wait()? {
                let mut err = String::new();
                let _ = stderr.read_to_string(&mut err).await;
                bail!(
                    "ssh master to {destination} exited {status}: {}",
                    err.trim()
                );
            }
            if std::time::Instant::now() > deadline {
                let _ = process.kill().await;
                bail!("ssh master to {destination}: control socket never appeared");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        Ok(Master {
            destination: destination.to_string(),
            socket,
            options: options.clone(),
            process,
        })
    }

    /// A mux-client command over the master's connection.
    fn client(&self) -> Command {
        let mut cmd = Command::new("ssh");
        cmd.arg("-S")
            .arg(&self.socket)
            .arg("-o")
            .arg("BatchMode=yes");
        apply_options(&mut cmd, &self.options);
        cmd.arg(&self.destination);
        cmd
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

    async fn close(mut self) {
        let _ = Command::new("ssh")
            .arg("-S")
            .arg(&self.socket)
            .arg("-O")
            .arg("exit")
            .arg(&self.destination)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        let _ = self.process.kill().await;
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn apply_options(cmd: &mut Command, options: &ConnectOptions) {
    if let Some(keyfile) = &options.keyfile {
        cmd.arg("-i")
            .arg(keyfile)
            .arg("-o")
            .arg("IdentitiesOnly=yes");
    }
    if options.accept_new_host_key {
        cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    }
}

/// One connected host: the master and the running agent's stdio stream.
pub struct AgentConnection {
    master: Master,
    agent: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
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
    check_mode: bool,
) -> Result<(AgentConnection, HostFacts)> {
    connect_with(
        destination,
        agent_binary,
        run_id,
        check_mode,
        &ConnectOptions::default(),
    )
    .await
}

pub async fn connect_with(
    destination: &str,
    agent_binary: &Path,
    run_id: &str,
    check_mode: bool,
    options: &ConnectOptions,
) -> Result<(AgentConnection, HostFacts)> {
    let master = Master::establish(destination, options).await?;

    let agent_bytes = std::fs::read(agent_binary)
        .with_context(|| format!("read agent binary {}", agent_binary.display()))?;
    let b3 = blake3::hash(&agent_bytes).to_hex().to_string();
    let remote_path = format!("{AGENT_DIR}/{b3}");

    let uploaded_agent = ensure_agent(&master, &remote_path, &agent_bytes).await?;

    let mut agent_cmd = master.client();
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
                ..Default::default()
            })),
        },
    )
    .await?;

    let ack_read = read_frame::<v1::Event>(&mut stdout);
    let event = match tokio::time::timeout(std::time::Duration::from_secs(30), ack_read).await {
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
            master,
            agent,
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
    /// Send one controller→agent message.
    pub async fn send(&mut self, envelope: &v1::Envelope) -> Result<()> {
        write_frame(&mut self.stdin, envelope).await
    }

    /// Receive the next agent event; `None` when the agent closed cleanly.
    pub async fn next_event(&mut self) -> Result<Option<v1::Event>> {
        read_frame(&mut self.stdout).await
    }

    /// Send Done, wait for the agent to flush and exit, close the master.
    pub async fn shutdown(mut self) -> Result<()> {
        write_frame(
            &mut self.stdin,
            &v1::Envelope {
                msg: Some(EnvMsg::Done(v1::Done {})),
            },
        )
        .await?;
        let status = self.agent.wait().await.context("agent exit")?;
        self.master.close().await;
        if !status.success() {
            bail!("agent exited with {status:?}");
        }
        Ok(())
    }
}

/// Content-addressed agent provisioning: if `remote_path` is already an
/// executable file, nothing moves; otherwise upload via an SFTP channel on
/// the same master and mark executable. Returns whether an upload happened.
async fn ensure_agent(master: &Master, remote_path: &str, bytes: &[u8]) -> Result<bool> {
    if master
        .exec_status(&["test", "-x", remote_path])
        .await?
        .success()
    {
        return Ok(false); // already provisioned at this hash
    }

    master
        .exec_status(&["mkdir", "-p", AGENT_DIR])
        .await?
        .success()
        .then_some(())
        .context("mkdir agent dir")?;

    // SFTP subsystem channel on the same master (ARCHITECTURE: ch1).
    let mut sftp_cmd = master.client();
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

    master
        .exec_status(&["chmod", "755", remote_path])
        .await?
        .success()
        .then_some(())
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
