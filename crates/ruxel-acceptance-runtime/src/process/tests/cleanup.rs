use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn oversized_stdout_and_stderr_trigger_bounded_cleanup() -> Result<()> {
    for redirect in ["", " >&2"] {
        let temp = tempfile::tempdir()?;
        let executable = temp.path().join("flood");
        std::fs::write(
            &executable,
            format!("#!/bin/sh\nexec /usr/bin/yes fixture-output{redirect}\n"),
        )?;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
        let started = Instant::now();
        let result = run(
            &executable,
            temp.path(),
            temp.path(),
            temp.path(),
            "apply",
            Duration::from_secs(5),
        );
        let error = result
            .err()
            .map(|error| format!("{error:#}"))
            .unwrap_or_default();
        ensure!(
            error.contains("captured output exceeded"),
            "oversized stream was not rejected: {error}"
        );
        ensure!(
            started.elapsed() < Duration::from_secs(5),
            "overflow cleanup was not prompt"
        );
    }
    Ok(())
}
