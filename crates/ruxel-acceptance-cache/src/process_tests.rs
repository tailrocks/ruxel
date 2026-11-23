use super::{run, run_ready};

use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn drains_both_pipes_before_exit() -> anyhow::Result<()> {
    let (status, stdout, stderr) = run(
        Command::new("/bin/sh")
            .args([
                "-c",
                "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2",
            ])
            .stdin(Stdio::null()),
        Duration::from_secs(5),
    )?;
    assert!(status.success());
    assert_eq!(stdout.len(), 131072);
    assert_eq!(stderr.len(), 131072);
    Ok(())
}

#[test]
fn descendant_holding_output_is_bounded() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let pid_file = temp.path().join("pid");
    let mut start = None;
    let result = run_ready(
        Command::new("/bin/sh")
            .args(["-c", "sleep 30 & echo $! > \"$1\"", "fixture"])
            .arg(&pid_file)
            .stdin(Stdio::null()),
        Duration::from_millis(500),
        |child| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !pid_file.exists() || child.try_wait()?.is_none() {
                anyhow::ensure!(Instant::now() < deadline, "fixture readiness timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
            let pid: i32 = std::fs::read_to_string(&pid_file)?.trim().parse()?;
            let pid = rustix::process::Pid::from_raw(pid)
                .ok_or_else(|| anyhow::anyhow!("invalid descendant pid"))?;
            rustix::process::test_kill_process(pid)?;
            start = Some(Instant::now());
            Ok(())
        },
    );
    assert!(result.is_err_and(|error| error.to_string().contains("timed out")));
    assert!(
        start
            .ok_or_else(|| anyhow::anyhow!("readiness not reached"))?
            .elapsed()
            < Duration::from_secs(5)
    );
    let pid = std::fs::read_to_string(pid_file)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let output = Command::new("/bin/ps")
            .args(["-o", "stat=", "-p", pid.trim()])
            .output()?;
        assert!(output.stderr.is_empty());
        let state = String::from_utf8(output.stdout)?;
        if state.trim().is_empty() || state.trim().starts_with('Z') {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "live descendant survived cleanup: {state}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
