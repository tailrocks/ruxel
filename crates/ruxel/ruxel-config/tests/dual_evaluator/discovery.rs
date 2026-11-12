//! The discovery of the official `pkl` the dual-evaluator coverage runs:
//! a sibling of `dual_evaluator.rs` within the one dual-evaluator target,
//! the repo `tests/schema.rs` + `tests/schema/` split pattern.

use super::*;

/// Prints the `pkl` this process discovers: the probe the discovery test runs in a child process, where its environment cannot race the tests above.
#[test]
fn the_discovery_probe_reports_the_pkl_it_found() -> Result {
    println!("{}", pkl()?.display());
    Ok(())
}

/// The discovery must find the binary Mise just installed: a redirected `MISE_DATA_DIR` (or `XDG_DATA_HOME`) is where `mise install pkl@0.33.0` puts it; a lookup that scans only `$HOME` fails with "not installed".
#[test]
fn the_discovery_finds_the_binary_mise_just_installed() -> Result {
    let scratch = std::env::temp_dir().join(format!(
        "ruxel-pkl-discovery-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let (data, home) = (scratch.join("mise-data"), scratch.join("home"));
    let installed = data.join("installs/pkl/0.33.0/pkl");
    std::fs::create_dir_all(
        installed
            .parent()
            .ok_or("the fake install path has no parent")?,
    )?;
    std::fs::create_dir_all(&home)?;
    std::fs::write(&installed, "#!/bin/sh\necho 'pkl 0.33.0'\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755))?;
    }
    let output = Command::new(std::env::current_exe()?)
        .args([
            "discovery::the_discovery_probe_reports_the_pkl_it_found",
            "--exact",
            "--nocapture",
        ])
        .env("MISE_DATA_DIR", &data)
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env_remove("XDG_DATA_HOME")
        .env_remove("RUXEL_PKL")
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains(&*installed.display().to_string()),
        "the probe must find the binary Mise just installed at {}; status: {}; stdout: {stdout}; stderr: {}",
        installed.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    drop(std::fs::remove_dir_all(&scratch));
    Ok(())
}
