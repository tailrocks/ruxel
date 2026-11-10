mod cache;
mod deploy;
mod local;
pub use local::LocalConnection;

use anyhow::{Context, Result, anyhow, bail};
use prost::Message;
use ruxel_proto::PROTO_VERSION;
use ruxel_proto::constants::{AGENT_DIR, HANDSHAKE_TIMEOUT_SECS};
use ruxel_proto::v1::{self, envelope::Msg as EnvMsg, event::Msg as EvMsg};
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
    /// The literal address admitted by `admit`. Every later invocation pins
    /// it, with proxies and connection multiplexing disabled, so a
    /// configuration change, `Match exec`, or an existing master cannot
    /// redirect the connection.
    admitted: Option<String>,
    #[cfg(test)]
    test_path: Option<std::ffi::OsString>,
}

impl RemoteHost {
    fn options(&self) -> Command {
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
        if let Some(address) = &self.admitted {
            // HostName expands % tokens; %% keeps a scope suffix literal.
            // An inherited multiplexing master could reach another endpoint.
            command
                .arg("-o")
                .arg(format!("HostName={}", address.replace('%', "%%")))
                .args([
                    "-o",
                    "ProxyJump=none",
                    "-o",
                    "ProxyCommand=none",
                    "-o",
                    "ControlMaster=no",
                    "-o",
                    "ControlPath=none",
                ]);
        }
        #[cfg(test)]
        if let Some(path) = &self.test_path {
            command.env("PATH", path);
        }
        command
    }
    fn client(&self) -> Command {
        let mut command = self.options();
        command.arg("--").arg(&self.destination);
        command
    }
    fn subsystem(&self) -> Command {
        let mut command = self.options();
        command.args(["-s", "--", &self.destination, "sftp"]);
        command
    }
    /// Resolve the destination with `ssh -G`, which reads configuration and
    /// makes no network contact, and refuse it unless it names a literal IP
    /// outside every deny entry with no proxy hop.
    async fn admit(&mut self, policy: &ruxel_guard::Policy) -> Result<()> {
        let mut command = self.options();
        command
            .args(["-G", "--", &self.destination])
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        let output = command.output().await.context("ssh -G")?;
        if !output.status.success() {
            bail!(
                "refused {}: ssh -G could not resolve the destination",
                self.destination
            );
        }
        let config = String::from_utf8(output.stdout).context("ssh -G output")?;
        self.admitted = Some(admit_resolved(&config, &self.destination, policy)?);
        Ok(())
    }
    async fn exec_status(&self, argv: &[&str]) -> Result<std::process::ExitStatus> {
        let mut cmd = self.client();
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
        for arg in argv {
            cmd.arg(arg);
        }
        cmd.stdin(Stdio::null()).stderr(Stdio::piped());
        cmd.output().await.context("ssh exec output")
    }
}

/// Returns the admitted hostname, including any IPv6 scope suffix.
fn admit_resolved(config: &str, destination: &str, policy: &ruxel_guard::Policy) -> Result<String> {
    let value = |key: &str| {
        config.lines().find_map(|line| {
            line.split_once(' ')
                .filter(|(name, _)| *name == key)
                .map(|(_, value)| value.trim())
        })
    };
    let hostname = value("hostname")
        .with_context(|| format!("refused {destination}: ssh -G reported no hostname"))?;
    let address = match hostname.split_once('%') {
        Some((address, scope)) if !scope.is_empty() => address
            .parse::<std::net::Ipv6Addr>()
            .ok()
            .map(std::net::IpAddr::V6),
        Some(_) => None,
        None => hostname.parse::<std::net::IpAddr>().ok(),
    }
    .ok_or_else(|| {
        anyhow!("refused {destination}: hostname {hostname:?} is not a literal IP address")
    })?;
    for key in ["proxyjump", "proxycommand"] {
        if value(key).is_some_and(|value| value != "none") {
            bail!("refused {destination}: {key} is set");
        }
    }
    policy
        .check_deny(address)
        .map_err(|rule| anyhow!("refused {destination} ({address}): {rule}"))?;
    Ok(hostname.to_owned())
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
        cmd.arg("-o").arg(known_hosts_option(kh));
    }
    cmd.arg("-o").arg(if options.accept_new_host_key {
        "StrictHostKeyChecking=accept-new"
    } else {
        "StrictHostKeyChecking=yes"
    });
}

fn known_hosts_option(path: &Path) -> std::ffi::OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let mut value = b"UserKnownHostsFile=\"".to_vec();
        for byte in path.as_os_str().as_bytes() {
            if matches!(byte, b'\\' | b'"') {
                value.push(b'\\');
            }
            value.push(*byte);
        }
        value.push(b'"');
        std::ffi::OsString::from_vec(value)
    }
    #[cfg(not(unix))]
    {
        let value = path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        std::ffi::OsString::from(format!("UserKnownHostsFile=\"{value}\""))
    }
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
    pub sensitive_effects: bool,
}

/// # Errors
/// Returns an error when the destination does not resolve to a literal IP
/// outside the compiled deny list, or when agent upload, verification, or
/// protocol negotiation fails.
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
        admitted: None,
        #[cfg(test)]
        test_path: None,
    };
    let policy = ruxel_guard::Policy::new(&[], &[])?;
    connect_remote(remote, &policy, agent_binary, run_id, check_mode).await
}

async fn connect_remote(
    mut remote: RemoteHost,
    policy: &ruxel_guard::Policy,
    agent_binary: &Path,
    run_id: &str,
    check_mode: bool,
) -> Result<(AgentConnection, HostFacts)> {
    remote.admit(policy).await?;
    let options = &remote.options;

    let agent_bytes = std::fs::read(agent_binary)
        .with_context(|| format!("read agent binary {}", agent_binary.display()))?;
    let b3 = blake3::hash(&agent_bytes).to_hex().to_string();
    let remote_path = format!("{AGENT_DIR}/{b3}");

    let uploaded_agent = deploy::ensure_agent(&remote, &remote_path, &agent_bytes).await?;

    let mut agent_cmd = remote.client();
    agent_cmd
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
        Err(_) => bail!("timed out waiting for HelloAck from {}", remote.destination),
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
            sensitive_effects: ack
                .sensitive_effects
                .context("agent omitted sensitivity state")?,
        },
    ))
}

impl AgentConnection {
    /// # Errors
    /// Returns protocol encoding or stream write errors.
    pub async fn send(&mut self, envelope: &v1::Envelope) -> Result<()> {
        write_frame(&mut self.stdin, envelope).await
    }

    /// # Errors
    /// Returns malformed frame or stream read errors.
    pub async fn next_event(&mut self) -> Result<Option<v1::Event>> {
        read_frame(&mut self.stdout).await
    }

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
