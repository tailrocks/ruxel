use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{ffi::OsString, path::Path, process::Command, time::Duration};

#[path = "docker_fixture/policy.rs"]
mod policy;
#[cfg(test)]
#[path = "docker_fixture/tests.rs"]
mod tests;

pub(super) struct Docker {
    program: std::path::PathBuf,
    config: tempfile::TempDir,
    name: String,
    image: String,
    id: Option<String>,
}

impl Docker {
    pub fn create(image: &str) -> Result<Self> {
        let program = if cfg!(target_os = "macos") {
            "/opt/homebrew/bin/docker"
        } else {
            "/usr/bin/docker"
        };
        Self::create_with_program(image, Path::new(program))
    }

    fn create_with_program(image: &str, program: &Path) -> Result<Self> {
        ensure!(
            policy::image_id(image),
            "fixture requires a complete local sha256 image ID"
        );
        let config = tempfile::Builder::new().prefix("ruxel-docker-").tempdir()?;
        use std::os::unix::ffi::OsStrExt;
        let name = format!(
            "ruxel-fixture-{}",
            crate::pins::digest(config.path().as_os_str().as_bytes())
        );
        let mut docker = Self {
            program: program.into(),
            config,
            name,
            image: image.into(),
            id: None,
        };
        let metadata = docker.checked(
            &[
                "image",
                "inspect",
                "--format",
                "{{json (index .Config \"Volumes\")}}",
                image,
            ],
            10,
        )?;
        ensure!(
            serde_json::from_slice::<Value>(&metadata)?.is_null(),
            "fixture image declares volumes"
        );
        let created = docker.checked_owned(&policy::create_args(&docker.name, image), 30);
        if let Ok(output) = &created
            && let Ok(id) = std::str::from_utf8(output)
            && policy::container_id(id.trim())
        {
            let id = id.trim();
            docker.id = Some(id.into());
        }
        let info = docker.inspect(&docker.name);
        match info {
            Ok(info) => {
                ensure!(
                    info["Config"]["Labels"]["org.ruxel.fixture"] == docker.name,
                    "fixture ownership mismatch"
                );
                let id = info["Id"].as_str().context("container ID")?;
                ensure!(policy::container_id(id), "invalid container ID");
                ensure!(
                    docker.id.as_deref().is_none_or(|created| created == id),
                    "create/inspect container mismatch"
                );
                docker.id = Some(id.into());
                policy::validate(&info, &docker.name, image, false)?;
            }
            Err(error) => {
                created?;
                return Err(error);
            }
        }
        created?;
        docker.checked_owned(&["start".into(), docker.id()?.into()], 15)?;
        docker.verify(true)?;
        Ok(docker)
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LC_ALL", "C")
            .args(["--host", "unix:///var/run/docker.sock", "--config"])
            .arg(self.config.path())
            .stdin(std::process::Stdio::null());
        cmd
    }
    fn checked(&self, args: &[&str], timeout: u64) -> Result<Vec<u8>> {
        self.checked_owned(
            &args.iter().map(OsString::from).collect::<Vec<_>>(),
            timeout,
        )
    }
    fn checked_owned(&self, args: &[OsString], timeout: u64) -> Result<Vec<u8>> {
        let (status, out, err) =
            crate::process::run(self.command().args(args), Duration::from_secs(timeout))?;
        ensure!(
            status.success(),
            "local fixture Docker command failed: {}",
            String::from_utf8_lossy(&err)
        );
        Ok(out)
    }
    fn inspect(&self, target: &str) -> Result<Value> {
        let template = concat!(
            "{\"Id\":{{json .Id}},\"Image\":{{json .Image}},\"State\":{{json .State}},",
            "\"Mounts\":{{json .Mounts}},\"HostConfig\":{{json .HostConfig}},\"Config\":{",
            "\"User\":{{json (index .Config \"User\")}},\"Labels\":{{json (index .Config \"Labels\")}},",
            "\"Volumes\":{{json (index .Config \"Volumes\")}},\"WorkingDir\":{{json (index .Config \"WorkingDir\")}},",
            "\"Healthcheck\":{{json (index .Config \"Healthcheck\")}},\"Entrypoint\":{{json (index .Config \"Entrypoint\")}},\"Cmd\":{{json (index .Config \"Cmd\")}}}}"
        );
        serde_json::from_slice(&self.checked(&["inspect", "--format", template, target], 10)?)
            .map_err(Into::into)
    }
    fn id(&self) -> Result<&str> {
        self.id.as_deref().context("fixture container absent")
    }
    fn verify(&self, running: bool) -> Result<()> {
        policy::validate(&self.inspect(self.id()?)?, &self.name, &self.image, running)
    }
    pub fn copy(&self, source: &Path, destination: &str) -> Result<()> {
        self.verify(true)?;
        ensure!(
            source.is_absolute() && !source.to_string_lossy().contains(':'),
            "invalid local copy source"
        );
        ensure!(
            [
                "/tmp/work/workload.pkl",
                "/tmp/work/assets/source.txt",
                "/tmp/work/assets/template.pkl",
                "/opt/ruxel-fixture/ruxel",
                "/opt/ruxel-fixture/ruxel-agent",
                "/opt/ruxel-fixture/systemctl"
            ]
            .contains(&destination),
            "invalid fixture copy destination"
        );
        let file = std::fs::File::open(source)?;
        ensure!(
            file.metadata()?.is_file(),
            "copy source is not a regular file"
        );
        let mut command = self.command();
        command
            .args([
                "exec",
                "--interactive",
                self.id()?,
                "/usr/bin/env",
                "-i",
                "/bin/dd",
            ])
            .arg(format!("of={destination}"))
            .args(["bs=65536", "status=none"])
            .stdin(file);
        let (status, _, _) = crate::process::run(&mut command, Duration::from_secs(30))?;
        ensure!(status.success(), "fixture input streaming failed");
        Ok(())
    }
    pub fn exec(&self, args: &[&str], expected: i32) -> Result<Vec<u8>> {
        self.verify(true)?;
        let mut command = self.command();
        command.args([
            "exec",
            "--workdir",
            "/tmp",
            self.id()?,
            "/usr/bin/env",
            "-i",
            "PATH=/usr/bin:/bin:/usr/sbin:/sbin",
            "HOME=/tmp",
            "TMPDIR=/tmp",
            "TMP=/tmp",
            "TEMP=/tmp",
            "LC_ALL=C",
        ]);
        command.args(args);
        let (status, out, err) = crate::process::run(&mut command, Duration::from_secs(60))?;
        ensure!(
            status.code() == Some(expected),
            "fixture command exit {status}: {}",
            String::from_utf8_lossy(&err)
        );
        Ok(out)
    }
    pub fn close(&mut self) -> Result<()> {
        if let Some(id) = &self.id {
            self.checked(&["rm", "--force", "--volumes", id], 15)?;
            self.id = None;
        }
        Ok(())
    }
}
impl Drop for Docker {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
