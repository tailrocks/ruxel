use crate::{process, runner::Env, state};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

fn mounts_safe(info: &str) -> bool {
    let mut overlay = false;
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
        if path == "/" {
            overlay = kind == "overlay";
        }
        if path == "/etc"
            || path.starts_with("/etc/")
                && !["/etc/hosts", "/etc/hostname", "/etc/resolv.conf"].contains(&path)
            || path == "/var"
            || path.starts_with("/var/")
        {
            return false;
        }
        if ["/tmp", "/work"].contains(&path) && kind != "tmpfs" {
            return false;
        }
        if path.starts_with("/tmp/") || path.starts_with("/work/") {
            return false;
        }
    }
    overlay
}
fn guard() -> Result<()> {
    ensure!(
        rustix::process::geteuid().as_raw() == 0
            && std::env::var("RUXEL_BECOME_FIXTURE").as_deref() == Ok("isolated")
            && Path::new("/.dockerenv").is_file(),
        "become fixture requires an isolated root Docker guest"
    );
    let interfaces = fs::read_dir("/sys/class/net")?
        .map(|e| e.map(|v| v.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    ensure!(
        interfaces.len() == 1 && interfaces[0] == "lo",
        "become fixture guest has external network interfaces"
    );
    ensure!(
        mounts_safe(&fs::read_to_string("/proc/self/mountinfo")?),
        "become fixture guest has unsafe mounts"
    );
    Ok(())
}
fn account() -> Result<Option<Vec<String>>> {
    let passwd = fs::read_to_string("/etc/passwd")?;
    let matches: Vec<_> = passwd
        .lines()
        .filter(|line| line.split(':').next() == Some("ruxelbecome"))
        .collect();
    ensure!(matches.len() <= 1, "duplicate fixture account");
    Ok(matches
        .first()
        .map(|line| line.split(':').map(str::to_owned).collect()))
}
fn verify_account(env: &Env, shell: &str) -> Result<()> {
    let fields = account()?.context("fixture account absent")?;
    ensure!(
        fields.len() == 7
            && fields[2].parse::<u32>()? >= 1000
            && fields[5]
                == env
                    .payload
                    .join("become-home")
                    .to_str()
                    .context("home path")?
            && fields[6] == shell,
        "fixture account mismatch"
    );
    Ok(())
}
fn drift(env: &Env) -> Result<()> {
    verify_account(env, "/bin/sh")?;
    let mut command = Command::new("/usr/sbin/usermod");
    command
        .args(["--shell", "/bin/false", "ruxelbecome"])
        .stdin(Stdio::null());
    let (status, _, _) = process::run(&mut command, Duration::from_secs(10))?;
    ensure!(status.success(), "fixture account drift failed");
    verify_account(env, "/bin/false")
}
fn verify_result(events: &[Value], home: &str) -> Result<()> {
    let events: Vec<_> = events
        .iter()
        .filter(|v| v["host"] == "fixture" && v["task"] == "Capture synthetic become environment")
        .collect();
    ensure!(
        events.len() == 1
            && events[0]["result"]["stdout_lines"]
                == json!([home, "ruxelbecome", "ruxelbecome", "/bin/sh", "synthetic"])
            && events[0]["result"]["failed"] == false,
        "become environment result mismatch"
    );
    Ok(())
}
fn verify_preview(events: &[Value], home: &str) -> Result<()> {
    ensure!(
        events.len() == 4
            && events[0]["host"] == "fixture"
            && events[0]["task"] == "Create synthetic become account"
            && events[0]["result"]["changed"] == false
            && events[0]["result"]["failed"] == false,
        "become preview account result mismatch"
    );
    verify_result(events, home)?;
    ensure!(
        events[1]["result"]["skipped"] != true,
        "become preview shell skipped"
    );
    ensure!(
        events[2]["host"] == "fixture"
            && events[2]["task"] == "Verify explicit become environment"
            && events[2]["result"]["failed"] == false,
        "become preview assertion mismatch"
    );
    ensure!(
        events[3]["host"] == "fixture"
            && events[3]["recap"]["failed"] == 0
            && events[3]["recap"]["skipped"] == 0,
        "become preview recap mismatch"
    );
    Ok(())
}
fn verify_drift_preview(events: &[Value], home: &str) -> Result<()> {
    ensure!(
        events.len() == 4
            && events[0]["result"]["changed"] == true
            && events[0]["result"]["failed"] == false,
        "drift preview account result mismatch"
    );
    ensure!(
        events[1]["host"] == "fixture"
            && events[1]["task"] == "Capture synthetic become environment"
            && events[1]["result"]["stdout_lines"]
                == json!([
                    home,
                    "ruxelbecome",
                    "ruxelbecome",
                    "/bin/false",
                    "synthetic"
                ])
            && events[1]["result"]["failed"] == false,
        "drift preview environment mismatch"
    );
    ensure!(
        events[2]["task"] == "Verify explicit become environment"
            && events[2]["result"]["failed"] == true
            && events[2]["result"]["msg"] == "Assertion failed"
            && events[3]["recap"]["failed"] == 1,
        "drift preview failure mismatch"
    );
    Ok(())
}
fn verify_drift_limitation(env: &Env, home: &str) -> Result<()> {
    drift(env)?;
    for mode in ["check", "diff"] {
        let etc = state::snapshot(Path::new("/etc"))?;
        let logs = state::snapshot(Path::new("/var/log"))?;
        let payload = state::snapshot(&env.payload)?;
        let fixture = state::fixture_snapshot(env.root.path())?;
        let events = env.run("become-environment", mode, 8)?;
        verify_drift_preview(&events, home)?;
        ensure!(
            state::snapshot(Path::new("/etc"))? == etc
                && state::snapshot(Path::new("/var/log"))? == logs
                && state::snapshot(&env.payload)? == payload
                && state::fixture_snapshot(env.root.path())? == fixture,
            "drift preview mutated state"
        );
        verify_account(env, "/bin/false")?;
        println!("LIMITATION become-environment drifted {mode}: assertion fails, state unchanged");
    }
    Ok(())
}
pub(super) fn execute(env: &Env) -> Result<()> {
    guard()?;
    ensure!(account()?.is_none(), "fixture account already exists");
    let home = env.payload.join("become-home");
    let home = home.to_str().context("home path")?;
    for phase in ["fresh", "converged", "drift_repair", "check", "diff"] {
        if phase == "drift_repair" {
            drift(env)?;
        }
        let preview = matches!(phase, "check" | "diff");
        let etc = state::snapshot(Path::new("/etc"))?;
        let logs = state::snapshot(Path::new("/var/log"))?;
        let payload = state::snapshot(&env.payload)?;
        let fixture = state::fixture_snapshot(env.root.path())?;
        let events = env.run(
            "become-environment",
            if preview { phase } else { "apply" },
            0,
        )?;
        if preview {
            verify_preview(&events, home)?;
            ensure!(
                state::snapshot(Path::new("/etc"))? == etc
                    && state::snapshot(Path::new("/var/log"))? == logs
                    && state::snapshot(&env.payload)? == payload
                    && state::fixture_snapshot(env.root.path())? == fixture,
                "become preview mutated state"
            );
            verify_account(env, "/bin/sh")?;
        } else {
            verify_account(env, "/bin/sh")?;
            ensure!(
                env.payload.join("become-home").is_dir(),
                "fixture home missing"
            );
            verify_result(&events, home)?;
        }
        println!("PASS become-environment {phase}");
    }
    verify_drift_limitation(env, home)
}
#[cfg(test)]
mod tests {
    use super::{mounts_safe, verify_preview};
    use serde_json::json;
    #[test]
    fn rejects_account_and_workspace_bind_mounts() {
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
        ] {
            assert!(!mounts_safe(&format!(
                "{root}\n3 1 0:3 / {path} rw - ext4 disk rw"
            )));
        }
        assert!(!mounts_safe("1 0 0:1 / / rw - ext4 disk rw"));
    }
    #[test]
    fn preview_requires_correct_environment_and_successful_assertion() -> anyhow::Result<()> {
        let mut events = vec![
            json!({"host":"fixture","task":"Create synthetic become account","result":{"changed":false,"failed":false}}),
            json!({"host":"fixture","task":"Capture synthetic become environment","result":{"failed":false,"stdout_lines":["/tmp/home","ruxelbecome","ruxelbecome","/bin/sh","synthetic"]}}),
            json!({"host":"fixture","task":"Verify explicit become environment","result":{"failed":false}}),
            json!({"host":"fixture","recap":{"failed":0,"skipped":0}}),
        ];
        verify_preview(&events, "/tmp/home")?;
        events[1]["result"]["skipped"] = json!(true);
        assert!(verify_preview(&events, "/tmp/home").is_err());
        Ok(())
    }
}
