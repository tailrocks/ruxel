use super::super::super::{ConnectOptions, RemoteHost, connect_remote};
use ruxel_proto::v1::{self, event::Msg as Event};
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

pub(super) struct Fixture {
    pub(super) root: PathBuf,
}
impl Fixture {
    pub(super) fn new() -> anyhow::Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ruxel-sftp-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(root.join("bin"))?;
        let ssh = root.join("bin/ssh");
        std::fs::write(&ssh, include_str!("fake_ssh.py"))?;
        std::fs::set_permissions(ssh, std::fs::Permissions::from_mode(0o700))?;
        std::fs::write(root.join("agent"), b"synthetic uploaded agent bytes")?;
        let ack = v1::Event {
            msg: Some(Event::HelloAck(v1::HelloAck {
                proto_version: ruxel_proto::PROTO_VERSION,
                facts: Some(v1::Facts {
                    hostname: "synthetic-agent".into(),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        };
        std::fs::write(root.join("ack"), ruxel_proto::frame::encode_frame(&ack))?;
        Ok(Self { root })
    }
    fn host(&self) -> anyhow::Result<RemoteHost> {
        let path = std::env::join_paths(std::iter::once(self.root.join("bin")).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))?;
        Ok(RemoteHost {
            destination: "fixture.invalid".into(),
            test_path: Some(path),
            options: ConnectOptions {
                keyfile: Some(self.root.join("identity key")),
                known_hosts_file: Some(self.root.join("known hosts")),
                port: Some(2222),
                diff_mode: true,
                no_cache: true,
                ..Default::default()
            },
        })
    }
    pub(super) async fn connect(
        &self,
    ) -> anyhow::Result<(
        super::super::super::AgentConnection,
        super::super::super::HostFacts,
    )> {
        connect_remote(
            self.host()?,
            &self.root.join("agent"),
            "deployment-test",
            true,
        )
        .await
    }
    pub(super) fn calls(&self) -> anyhow::Result<Vec<Value>> {
        std::fs::read_to_string(self.root.join("calls.jsonl"))?
            .lines()
            .map(|line| Ok(serde_json::from_str(line)?))
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            eprintln!("remove synthetic transport fixture: {error}");
        }
    }
}
