use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn timeout_stops_a_stalled_cli() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let executable = temp.path().join("stalled");
    std::fs::write(&executable, "#!/bin/sh\nexec sleep 30\n")?;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
    let started = Instant::now();
    let result = run(
        &executable,
        temp.path(),
        temp.path(),
        temp.path(),
        "apply",
        Duration::from_millis(100),
    );
    ensure!(
        result.is_err_and(|error| error.to_string().contains("timed out")),
        "timeout not reported"
    );
    ensure!(
        started.elapsed() < Duration::from_secs(5),
        "timeout did not stop the process promptly"
    );
    Ok(())
}

#[test]
fn timeout_stops_descendant_before_delayed_write() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let executable = temp.path().join("stalled");
    let started = temp.path().join("started");
    let marker = temp.path().join("marker");
    let release = temp.path().join("release");
    let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\n(printf x > {}; while [ ! -f {} ]; do sleep 0.01; done; printf leaked > {}) &\nwait\n",
            quote(&started),
            quote(&release),
            quote(&marker)
        ),
    )?;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
    let result = run(
        &executable,
        temp.path(),
        temp.path(),
        temp.path(),
        "apply",
        Duration::from_secs(1),
    );
    std::fs::write(&release, "release")?;
    ensure!(
        result.is_err_and(|error| error.to_string().contains("timed out")),
        "timeout not reported"
    );
    ensure!(started.is_file(), "fixture did not start before timeout");
    thread::sleep(Duration::from_millis(2300));
    ensure!(
        !marker.exists(),
        "descendant survived timeout and wrote a marker"
    );
    Ok(())
}
