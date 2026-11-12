//! `tools/verify-pkl-artifact` at the script level: the verifier must
//! reach the binary under every Mise data directory it names, whatever
//! characters the path carries.
//!
//! Each test runs the script itself in a child process against a fresh
//! scratch tree, the way the dual-evaluator discovery is covered
//! (`crates/ruxel/ruxel-config/tests/dual_evaluator.rs`): a script's
//! behavior is its output and its exit status. The fake install is never
//! the official binary, so a verifier that reaches it answers with the
//! checksum refusal — the observable proof that the lookup found it.

use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The verifier, at the repository root.
fn verifier() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tools/verify-pkl-artifact"
    ))
    .to_path_buf()
}

/// A fresh scratch directory this test owns.
fn scratch(name: &str) -> Result<PathBuf> {
    let directory = std::env::temp_dir().join(format!(
        "ruxel-verify-pkl-artifact-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

/// A fake pkl 0.33.0 install under `data`: a file at the exact path the
/// verifier scans first. It is not the official binary, so a verifier
/// that reaches it refuses it on its checksum.
fn fake_install(data: &Path) -> Result<PathBuf> {
    let installed = data.join("installs/pkl/0.33.0/pkl");
    std::fs::create_dir_all(
        installed
            .parent()
            .ok_or("the fake install path has no parent")?,
    )?;
    std::fs::write(&installed, "#!/bin/sh\necho 'pkl 0.33.0'\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(installed)
}

/// Whether the verifier reached the fake: it must refuse it on its
/// checksum, naming the binary it hashed. A lookup that never found the
/// binary answers "no pkl 0.33.0 install found" instead.
fn assert_reached(output: &std::process::Output, installed: &Path) -> Result {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reached = format!("{} is SHA-256 ", installed.display());
    if output.status.success() || !stderr.contains(&*reached) {
        return Err(format!(
            "the verifier must reach the fake at {} and refuse it on its checksum; status: {}; stderr: {stderr}",
            installed.display(),
            output.status
        )
        .into());
    }
    Ok(())
}

/// A `MISE_DATA_DIR` that carries whitespace is one directory, not a
/// word list: the verifier must scan it whole and reach the binary Mise
/// installed there.
#[test]
fn the_verifier_reaches_the_binary_under_a_mise_data_dir_with_whitespace() -> Result {
    let scratch = scratch("mise-data-dir")?;
    let data = scratch.join("mise data");
    let installed = fake_install(&data)?;
    let home = scratch.join("home");
    std::fs::create_dir_all(&home)?;
    let output = Command::new("sh")
        .arg(verifier())
        .env("MISE_DATA_DIR", &data)
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env_remove("XDG_DATA_HOME")
        .output()?;
    assert_reached(&output, &installed)?;
    drop(std::fs::remove_dir_all(&scratch));
    Ok(())
}

/// The same for `XDG_DATA_HOME`: the verifier appends `mise` to the
/// configured directory, and the joined path keeps its whitespace.
#[test]
fn the_verifier_reaches_the_binary_under_an_xdg_data_home_with_whitespace() -> Result {
    let scratch = scratch("xdg-data-home")?;
    let data = scratch.join("xdg data").join("mise");
    let installed = fake_install(&data)?;
    let home = scratch.join("home");
    std::fs::create_dir_all(&home)?;
    let output = Command::new("sh")
        .arg(verifier())
        .env("XDG_DATA_HOME", scratch.join("xdg data"))
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env_remove("MISE_DATA_DIR")
        .output()?;
    assert_reached(&output, &installed)?;
    drop(std::fs::remove_dir_all(&scratch));
    Ok(())
}

/// And for the `$HOME` default: `mise install pkl@0.33.0` puts the
/// binary under `$HOME/.local/share/mise`, and a home directory with
/// whitespace must be scanned as one directory too.
#[test]
fn the_verifier_reaches_the_binary_under_a_home_with_whitespace() -> Result {
    let scratch = scratch("home")?;
    let home = scratch.join("the home");
    let installed = fake_install(&home.join(".local/share/mise"))?;
    std::fs::create_dir_all(&home)?;
    let output = Command::new("sh")
        .arg(verifier())
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env_remove("MISE_DATA_DIR")
        .env_remove("XDG_DATA_HOME")
        .output()?;
    assert_reached(&output, &installed)?;
    drop(std::fs::remove_dir_all(&scratch));
    Ok(())
}
