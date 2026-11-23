use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn completed_cli_does_not_leave_descendants_running() -> Result<()> {
    for code in [0, 1] {
        let temp = tempfile::tempdir()?;
        let executable = temp.path().join("exits");
        let started = temp.path().join("started");
        let marker = temp.path().join("marker");
        let release = temp.path().join("release");
        let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\n(printf x > {}; while [ ! -f {} ]; do sleep 0.01; done; printf leaked > {}) &\nwhile [ ! -f {} ]; do sleep 0.01; done\nsleep 1\nprintf '{{}}\\n'\nexit {code}\n",
                quote(&started),
                quote(&release),
                quote(&marker),
                quote(&started)
            ),
        )?;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
        let outcome = run(
            &executable,
            temp.path(),
            temp.path(),
            temp.path(),
            "apply",
            Duration::from_secs(5),
        );
        std::fs::write(&release, "release")?;
        ensure!(started.is_file(), "background fixture did not start");
        if code == 0 {
            outcome?;
        } else {
            let error = outcome.expect_err("exit 1 must fail").to_string();
            ensure!(error.starts_with("apply exited exit status: 1:"), "{error}");
        }
        thread::sleep(Duration::from_millis(800));
        ensure!(!marker.exists(), "descendant survived completed CLI");
    }
    Ok(())
}
