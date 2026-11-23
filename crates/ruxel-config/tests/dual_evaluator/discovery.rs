//! The resolution of the official `pkl` the dual-evaluator coverage runs:
//! a sibling of `dual_evaluator.rs` within the one dual-evaluator target,
//! the repo `tests/schema.rs` + `tests/schema/` split pattern.
//!
//! The pair covers the one resolver verification and execution share:
//! `RUXEL_PKL`, the absolute path `tools/verify-pkl-artifact` hashed and
//! printed and the mise `dual-evaluator` task exports to the suite. A
//! `pkl` earlier on `PATH` is never a candidate: the artifact the
//! verifier hashed is the only binary the suite may execute, and where no
//! verified path is supplied the suite refuses to run instead of
//! resolving a binary of its own.

use super::*;

/// Prints the `pkl` this process resolves: the probe the tests below run
/// in a child process, where its environment cannot race the tests above.
#[test]
fn the_resolution_probe_reports_the_pkl_it_resolved() -> Result {
    println!("{}", pkl()?.display());
    Ok(())
}

/// A scratch tree holding the two binaries the resolution must choose
/// between: `installed`, where `mise install pkl@0.33.0` puts the
/// artifact the verifier hashes, and `tampered`, a different working
/// `pkl` earlier on `PATH`.
///
/// The name carries a sequence number alongside the pid: the tests below
/// run in parallel within one process, and a name of pid and timestamp
/// alone collided when two of them asked for a scratch tree within one
/// clock tick — sharing one tree, whose cleanup the first finisher ran
/// under the other's still-running child probe.
fn scratch() -> Result<(PathBuf, PathBuf, PathBuf)> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let scratch = std::env::temp_dir().join(format!(
        "ruxel-pkl-resolution-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let installed = scratch.join("mise-data/installs/pkl/0.33.0/pkl");
    let tampered = scratch.join("bin/pkl");
    for binary in [&installed, &tampered] {
        std::fs::create_dir_all(
            binary
                .parent()
                .ok_or("the scratch binary path has no parent")?,
        )?;
        std::fs::write(binary, "#!/bin/sh\necho 'pkl 0.33.0'\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    std::fs::create_dir_all(scratch.join("home"))?;
    Ok((scratch, installed, tampered))
}

/// Runs the probe in a child process over the scratch tree: `installed`
/// is where Mise keeps the artifact, `bin` precedes everything else on
/// `PATH`, and `verified` is the `RUXEL_PKL` the child receives (or its
/// absence).
fn probe(scratch: &Path, verified: Option<&Path>) -> Result<std::process::Output> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "discovery::the_resolution_probe_reports_the_pkl_it_resolved",
            "--exact",
            "--nocapture",
        ])
        .env("MISE_DATA_DIR", scratch.join("mise-data"))
        .env("HOME", scratch.join("home"))
        .env("PATH", format!("{}/bin:/usr/bin:/bin", scratch.display()))
        .env_remove("XDG_DATA_HOME")
        .env_remove("RUXEL_PKL");
    if let Some(verified) = verified {
        command.env("RUXEL_PKL", verified);
    }
    Ok(command.output()?)
}

/// The suite refuses to run where no verified path was supplied, even
/// though a working `pkl` stands first on `PATH` and the Mise install the
/// verifier hashes is present too: the PATH-first candidates must no
/// longer win, or the suite would execute a binary that is not the one
/// `tools/verify-pkl-artifact` hashed.
#[test]
fn no_path_pkl_stands_in_for_the_unsupplied_verified_one() -> Result {
    let (scratch, installed, _tampered) = scratch()?;
    let output = probe(&scratch, None)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() && stderr.contains("RUXEL_PKL"),
        "the probe must refuse to resolve a pkl of its own when no verified path is supplied, \
         though the Mise install at {} and a tampered pkl earlier on PATH both answer; \
         status: {}; stdout: {stdout}; stderr: {stderr}",
        installed.display(),
        output.status
    );
    drop(std::fs::remove_dir_all(&scratch));
    Ok(())
}

/// And where the verified path is supplied, the suite executes that
/// artifact and no other: a different, tampered `pkl` earlier on `PATH`
/// must never displace the binary `tools/verify-pkl-artifact` hashed.
#[test]
fn the_verified_artifact_displaces_an_earlier_path_pkl() -> Result {
    let (scratch, installed, _tampered) = scratch()?;
    let output = probe(&scratch, Some(&installed))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains(&*installed.display().to_string()),
        "the probe must resolve the verified artifact at {}; status: {}; stdout: {stdout}; stderr: {}",
        installed.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    drop(std::fs::remove_dir_all(&scratch));
    Ok(())
}
