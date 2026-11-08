use crate::process;
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

fn checked(mut command: Command, limit: u64) -> Result<Vec<u8>> {
    crate::isolation::configure(&mut command);
    let (status, stdout, _) = process::run(&mut command, Duration::from_secs(limit))?;
    ensure!(status.success(), "isolated postgres command failed");
    Ok(stdout)
}
fn as_postgres(binary: &Path) -> Command {
    let mut command = Command::new("/usr/sbin/runuser");
    command
        .args(["-u", "postgres", "--"])
        .arg(binary)
        .stdin(Stdio::null());
    clear_pg(&mut command);
    command
}
pub(super) fn clear_pg(command: &mut Command) {
    for key in [
        "PGHOST",
        "PGHOSTADDR",
        "PGPORT",
        "PGUSER",
        "PGDATABASE",
        "PGPASSWORD",
        "PGSERVICE",
        "PGSERVICEFILE",
        "PGOPTIONS",
        "PGPASSFILE",
    ] {
        command.env_remove(key);
    }
}
fn safe_socket_path(path: &str) -> bool {
    path.starts_with('/')
        && path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/._-".contains(&c))
}
pub(super) struct Cluster {
    pub socket: PathBuf,
    data: PathBuf,
    ctl: PathBuf,
    started: bool,
}
impl Cluster {
    pub fn start(root: &Path) -> Result<Self> {
        crate::isolation::guard("RUXEL_POSTGRES_FIXTURE")?;
        ensure!(
            root.parent() == Some(Path::new("/tmp")),
            "postgres fixture is outside private tmpfs"
        );
        let mut config = Command::new("/usr/bin/pg_config");
        config.arg("--bindir").stdin(Stdio::null());
        let bindir = String::from_utf8(checked(config, 5)?)?;
        let bindir = Path::new(bindir.trim());
        ensure!(
            bindir.starts_with("/usr/lib/postgresql") && bindir.is_absolute(),
            "unexpected postgres binary directory"
        );
        let path = root.join("pg");
        let socket = path.join("socket");
        let data = path.join("data");
        fs::set_permissions(root, fs::Permissions::from_mode(0o711))?;
        fs::create_dir(&path)?;
        fs::create_dir(&socket)?;
        fs::create_dir(&data)?;
        let mut chown = Command::new("/usr/bin/chown");
        chown
            .args(["-R", "postgres:postgres"])
            .arg(&path)
            .stdin(Stdio::null());
        checked(chown, 5)?;
        let mut init = as_postgres(&bindir.join("initdb"));
        init.arg("-D")
            .arg(&data)
            .args(["--auth-local=trust", "--no-instructions", "--no-locale"]);
        checked(init, 30)?;
        let mut cluster = Self {
            socket,
            data,
            ctl: bindir.join("pg_ctl"),
            started: false,
        };
        let socket_str = cluster.socket.to_str().context("non-UTF8 socket")?;
        ensure!(safe_socket_path(socket_str), "unsafe socket path");
        let mut start = as_postgres(&cluster.ctl);
        start
            .arg("-D")
            .arg(&cluster.data)
            .arg("-l")
            .arg(path.join("postgres.log"))
            .args(["-w", "-t", "20", "-o"])
            .arg(format!("-k {socket_str} -p 40000 -h ''"))
            .arg("start");
        cluster.started = true;
        checked(start, 25)?;
        Ok(cluster)
    }
    pub fn query(&self, sql: &str) -> Result<String> {
        let mut cmd = as_postgres(Path::new("/usr/bin/psql"));
        cmd.args(["-X", "-qAt", "-v", "ON_ERROR_STOP=1", "-h"])
            .arg(&self.socket)
            .args(["-p", "40000", "-U", "postgres", "-d", "postgres", "-c", sql]);
        Ok(String::from_utf8(checked(cmd, 10)?)?)
    }
    pub fn stop(&mut self) -> Result<()> {
        if self.started {
            let mut command = as_postgres(&self.ctl);
            command
                .arg("-D")
                .arg(&self.data)
                .args(["-m", "immediate", "-w", "-t", "10", "stop"]);
            checked(command, 12)?;
            self.started = false;
        }
        Ok(())
    }
}

impl Drop for Cluster {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::safe_socket_path;
    #[test]
    fn rejects_shell_metacharacters_in_postgres_socket_path() {
        assert!(safe_socket_path("/tmp/ruxel-pg/socket"));
        for invalid in [
            "relative/path",
            "/tmp/x;touch",
            "/tmp/$(command)",
            "/tmp/a b",
            "/tmp/a'bad",
        ] {
            assert!(!safe_socket_path(invalid));
        }
    }
}
