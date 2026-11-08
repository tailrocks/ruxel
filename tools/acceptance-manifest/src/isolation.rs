use anyhow::{Result, ensure};
use std::{fs, path::Path, process::Command};

fn mounts_safe(info: &str) -> bool {
    let mut overlay = false;
    let mut tmpfs = false;
    for line in info.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            return false;
        };
        let Some(path) = left.split_whitespace().nth(4) else {
            return false;
        };
        let Some(kind) = right.split_whitespace().next() else {
            return false;
        };
        let safe = match path {
            "/" => {
                overlay = kind == "overlay";
                overlay
            }
            "/tmp" => {
                tmpfs = kind == "tmpfs";
                tmpfs
            }
            "/work" | "/dev" | "/dev/shm" => kind == "tmpfs",
            "/dev/pts" => kind == "devpts",
            "/dev/mqueue" => kind == "mqueue",
            "/proc" => kind == "proc",
            "/sys" => kind == "sysfs",
            "/sys/fs/cgroup" => kind == "cgroup2",
            "/sys/firmware" => kind == "tmpfs",
            "/etc/hosts" | "/etc/hostname" | "/etc/resolv.conf" => true,
            _ if path.starts_with("/proc/") => kind == "proc" || kind == "tmpfs",
            _ => false,
        };
        if !safe {
            return false;
        }
    }
    overlay && tmpfs
}
pub(super) fn guard(marker: &str) -> Result<()> {
    ensure!(
        rustix::process::geteuid().as_raw() == 0
            && std::env::var(marker).as_deref() == Ok("isolated")
            && Path::new("/.dockerenv").is_file(),
        "fixture requires an isolated root Docker guest"
    );
    let interfaces = fs::read_dir("/sys/class/net")?
        .map(|e| e.map(|v| v.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    ensure!(
        interfaces.len() == 1 && interfaces[0] == "lo",
        "fixture guest has external network interfaces"
    );
    ensure!(
        mounts_safe(&fs::read_to_string("/proc/self/mountinfo")?),
        "fixture guest has unsafe mounts"
    );
    ensure!(
        fs::symlink_metadata("/tmp")?.file_type().is_dir(),
        "fixture temporary directory is not a directory"
    );
    Ok(())
}
pub(super) fn configure(command: &mut Command) {
    command
        .env("TMPDIR", "/tmp")
        .env("TMP", "/tmp")
        .env("TEMP", "/tmp")
        .env("HOME", "/tmp");
}
#[cfg(test)]
mod tests {
    use super::{configure, mounts_safe};
    use std::process::Command;
    #[test]
    fn rejects_unapproved_mounts_and_requires_private_tmpfs() {
        let root = "1 0 0:1 / / rw - overlay overlay rw\n2 1 0:2 / /tmp rw - tmpfs tmpfs rw";
        assert!(mounts_safe(root));
        for path in [
            "/etc",
            "/etc/passwd",
            "/etc/shadow",
            "/var",
            "/var/log",
            "/tmp/host",
            "/work/host",
            "/external",
            "/home",
        ] {
            assert!(!mounts_safe(&format!(
                "{root}\n3 1 0:3 / {path} rw - ext4 disk rw"
            )));
            assert!(!mounts_safe(&format!(
                "{root}\n3 1 0:3 / {path} rw - tmpfs tmpfs rw"
            )));
        }
        assert!(!mounts_safe("1 0 0:1 / / rw - overlay overlay rw"));
        assert!(!mounts_safe(
            "1 0 0:1 / / rw - overlay overlay rw\n2 1 0:2 / /tmp rw - ext4 disk rw"
        ));
        assert!(!mounts_safe(
            "1 0 0:1 / / rw - ext4 disk rw\n2 1 0:2 / /tmp rw - tmpfs tmpfs rw"
        ));
    }
    #[test]
    fn subprocess_temporaries_override_untrusted_environment() {
        let mut command = Command::new("unused");
        for key in ["TMPDIR", "TMP", "TEMP", "HOME"] {
            command.env(key, "/host/bind");
        }
        configure(&mut command);
        let mut checked = 0;
        for (key, value) in command.get_envs() {
            if ["TMPDIR", "TMP", "TEMP", "HOME"]
                .iter()
                .any(|wanted| key == *wanted)
            {
                assert_eq!(value.and_then(|value| value.to_str()), Some("/tmp"));
                checked += 1;
            }
        }
        assert_eq!(checked, 4);
    }
}
