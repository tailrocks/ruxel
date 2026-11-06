//! A local agent subprocess for explicitly selected disposable test environments.
use super::{read_frame, write_frame};
use anyhow::{Context, Result, bail};
use ruxel_proto::{PROTO_VERSION, v1};
use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// An actual agent process connected through its framed protocol.
pub struct LocalConnection {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl LocalConnection {
    /// Launch an agent locally. This performs no SSH or upload operations.
    /// The caller must select safe disposable paths and commands.
    pub async fn connect(
        binary: &Path,
        state: &Path,
        run_id: &str,
        check: bool,
        diff: bool,
    ) -> Result<(Self, v1::Facts)> {
        let mut child = Command::new(binary)
            .env("RUXEL_STATE_DIR", state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("start local agent")?;
        let stdin = child.stdin.take().context("local agent stdin")?;
        let stdout = child.stdout.take().context("local agent stdout")?;
        let mut connection = Self {
            child,
            stdin,
            stdout,
        };
        connection
            .send(&v1::Envelope {
                msg: Some(v1::envelope::Msg::Hello(v1::Hello {
                    proto_version: PROTO_VERSION,
                    run_id: run_id.into(),
                    check_mode: check,
                    diff_mode: diff,
                    no_cache: true,
                })),
            })
            .await?;
        let event =
            tokio::time::timeout(std::time::Duration::from_secs(15), connection.next_event())
                .await
                .context("local agent handshake timeout")??;
        match event.and_then(|event| event.msg) {
            Some(v1::event::Msg::HelloAck(ack)) if ack.proto_version == PROTO_VERSION => {
                Ok((connection, ack.facts.unwrap_or_default()))
            }
            _ => bail!("local agent refused protocol handshake"),
        }
    }

    /// Send a framed request.
    pub async fn send(&mut self, envelope: &v1::Envelope) -> Result<()> {
        write_frame(&mut self.stdin, envelope).await
    }
    /// Read the next framed event.
    pub async fn next_event(&mut self) -> Result<Option<v1::Event>> {
        read_frame(&mut self.stdout).await
    }
    /// Flush the agent ledger and wait for clean termination.
    pub async fn shutdown(mut self) -> Result<()> {
        self.send(&v1::Envelope {
            msg: Some(v1::envelope::Msg::Done(v1::Done {})),
        })
        .await?;
        let status = self.child.wait().await.context("wait for local agent")?;
        if !status.success() {
            bail!("local agent exited unsuccessfully");
        }
        Ok(())
    }
}
