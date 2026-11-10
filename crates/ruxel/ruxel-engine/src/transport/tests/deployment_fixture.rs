use super::super::super::{ConnectOptions, RemoteHost, connect_remote};
use prost::Message;
use ruxel_proto::v1::envelope::Msg;
use ruxel_proto::v1::{self, event::Msg as Event};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};

fn native_fixture() -> anyhow::Result<&'static [u8]> {
    static BINARY: OnceLock<Result<Vec<u8>, String>> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let compile = || -> anyhow::Result<Vec<u8>> {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos();
                let directory = std::env::temp_dir()
                    .join(format!("ruxel-native-ssh-{}-{stamp}", std::process::id()));
                std::fs::create_dir(&directory)?;
                let build = (|| -> anyhow::Result<Vec<u8>> {
                    let output = directory.join("fixture");
                    let source = directory.join("fake_ssh.rs");
                    std::fs::write(&source, include_str!("fake_ssh.rs"))?;
                    std::fs::write(directory.join("fake_sftp.rs"), include_str!("fake_sftp.rs"))?;
                    let status = std::process::Command::new(
                        std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()),
                    )
                    .args(["--edition=2024", "-Dwarnings"])
                    .arg(source)
                    .arg("-o")
                    .arg(&output)
                    .output()?;
                    anyhow::ensure!(
                        status.status.success(),
                        "compile native SSH fixture: {}",
                        String::from_utf8_lossy(&status.stderr)
                    );
                    Ok(std::fs::read(output)?)
                })();
                std::fs::remove_dir_all(directory)?;
                build
            };
            compile().map_err(|error| format!("{error:#}"))
        })
        .as_deref()
        .map_err(|error| anyhow::anyhow!(error.clone()))
}

pub(super) struct Fixture {
    pub(super) root: PathBuf,
}
impl Fixture {
    pub(super) fn new() -> anyhow::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ruxel-sftp-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&root)?;
        std::fs::create_dir(root.join("bin"))?;
        std::fs::create_dir_all(root.join("remote/var/lib"))?;
        for directory in ["remote", "remote/var", "remote/var/lib"] {
            std::fs::set_permissions(root.join(directory), std::fs::Permissions::from_mode(0o755))?;
        }
        let ssh = root.join("bin/ssh");
        std::fs::write(&ssh, native_fixture()?)?;
        std::fs::set_permissions(ssh, std::fs::Permissions::from_mode(0o700))?;
        std::fs::write(root.join("agent"), b"synthetic uploaded agent bytes")?;
        let mut digest = String::new();
        for byte in Sha256::digest(b"synthetic uploaded agent bytes") {
            write!(digest, "{byte:02x}")?;
        }
        std::fs::write(root.join("digest"), digest)?;
        let ack = v1::Event {
            msg: Some(Event::HelloAck(v1::HelloAck {
                proto_version: ruxel_proto::PROTO_VERSION,
                sensitive_effects: Some(false),
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
    pub(super) fn override_owner(
        &self,
        path: &str,
        uid: u32,
        inode: Option<u64>,
    ) -> anyhow::Result<()> {
        std::fs::write(
            self.root.join("metadata"),
            format!(
                "{path}\t{uid}\t{}\n",
                inode.map_or_else(|| "*".into(), |value| value.to_string())
            ),
        )?;
        Ok(())
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
            &ruxel_guard::Policy::new(&[], &["192.0.2.66/32".into()])?,
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
    pub(super) fn verify_first_upload(&self) -> anyhow::Result<std::path::PathBuf> {
        let calls = self.calls()?;
        let remote = calls.iter().find(|event| event[0] == "launch").unwrap()[1]
            .as_str()
            .unwrap();
        let cache = self.root.join("remote/var/lib/ruxel/agent");
        let uploaded = self
            .root
            .join("remote")
            .join(remote.trim_start_matches('/'));
        assert_eq!(
            std::fs::read(&uploaded)?,
            std::fs::read(self.root.join("agent"))?
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
        let hello = v1::Envelope::decode(std::fs::read(self.root.join("hello"))?.as_slice())?;
        assert!(
            matches!(hello.msg, Some(Msg::Hello(h)) if h.run_id == "deployment-test" && h.check_mode && h.diff_mode && h.no_cache)
        );
        let done = v1::Envelope::decode(std::fs::read(self.root.join("done"))?.as_slice())?;
        assert!(matches!(done.msg, Some(Msg::Done(_))));
        Ok(uploaded)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            eprintln!("remove synthetic transport fixture: {error}");
        }
    }
}

#[test]
fn concurrent_fixtures_have_distinct_roots() -> anyhow::Result<()> {
    let barrier = std::sync::Barrier::new(16);
    let fixtures = std::thread::scope(|scope| {
        let barrier = &barrier;
        let threads = (0..16)
            .map(|_| {
                scope.spawn(move || {
                    barrier.wait();
                    Fixture::new()
                })
            })
            .collect::<Vec<_>>();
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<anyhow::Result<Vec<_>>>()
    })?;
    let roots = fixtures
        .iter()
        .map(|fixture| &fixture.root)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(roots.len(), fixtures.len());
    Ok(())
}
