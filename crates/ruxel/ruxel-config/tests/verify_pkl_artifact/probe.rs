//! The executability probe of `tools/verify-pkl-artifact`
//! (thread `PRRT_kwDOS3Fimc62gU0W`): a checksum proves the bytes, not
//! that the binary can start, so a candidate that hashes to the
//! committed checksum but cannot execute — a mode bit lost in a manual
//! cache restore, a cache directory mounted noexec — must be passed
//! over, the remaining candidates still tried, and never returned as
//! `RUXEL_PKL`.
//!
//! The cache entry has to genuinely match the committed checksum for
//! the probe branch to be reached at all, and a real artifact cannot
//! be forged — so this test runs a copy of the verifier beside a
//! doctored copy of `mise.lock` whose linux-x64 checksum is the digest
//! of the stub payload. The script under test is byte-identical to the
//! committed one; only the lockfile fixture differs.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{fake_curl, fake_ldd, fake_uname, run_script, scratch, verifier};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The committed linux-x64 checksum, the one the fixture replaces.
const COMMITTED_LINUX_X64: &str =
    "b9a6ddea6b382b7d4db4cf78e77aa3858e0c8a7e02939d2ba298574b5d1fcc69";

/// The SHA-256 of a file, through the same hashers the verifier tries.
fn sha256_of(path: &Path) -> Result<String> {
    let text = match Command::new("sha256sum").arg(path).output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => {
            let out = Command::new("shasum")
                .arg("-a")
                .arg("256")
                .arg(path)
                .output()?;
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
    };
    Ok(text
        .split_whitespace()
        .next()
        .ok_or("the hasher printed no digest")?
        .to_string())
}

/// A scratch tree carrying the committed verifier script beside a
/// doctored lockfile: a copy of the repository's `mise.lock` whose
/// linux-x64 checksum is `digest`, so the stub payload the test caches
/// genuinely matches the committed checksum the copy declares.
fn tree_with_doctored_lock(digest: &str) -> Result<PathBuf> {
    let tree = scratch("non-exec-cache")?;
    let tools = tree.join("tools");
    std::fs::create_dir_all(&tools)?;
    std::fs::copy(verifier(), tools.join("verify-pkl-artifact"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            tools.join("verify-pkl-artifact"),
            std::fs::Permissions::from_mode(0o755),
        )?;
    }
    let committed =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../mise.lock"))?;
    std::fs::write(
        tree.join("mise.lock"),
        committed.replace(COMMITTED_LINUX_X64, digest),
    )?;
    Ok(tree)
}

/// A checksum-matching but non-executable cache entry is passed over,
/// never returned: the run reports the probe failure, still tries the
/// remaining candidates (the fetch — unreachable here, exit 22), and
/// ends with the missing verdict of exit 1 and nothing on stdout. With
/// the execute mode withheld the probe cannot start the file at all,
/// which is the lost-mode-bit shape; a noexec-mounted cache directory
/// fails the same probe with the mode present.
#[test]
fn a_non_executable_cached_artifact_is_passed_over() -> Result {
    let payload_tree = scratch("payload")?;
    let payload = payload_tree.join("payload");
    std::fs::write(&payload, "not the official pkl binary\n")?;
    let digest = sha256_of(&payload)?;
    let tree = tree_with_doctored_lock(&digest)?;

    // The cache entry: the payload at the checksum-keyed path the
    // verifier computes, with its execute mode withheld.
    let cached = tree
        .join("cache with space")
        .join(&digest)
        .join("pkl-linux-amd64");
    std::fs::create_dir_all(cached.parent().ok_or("no cache parent")?)?;
    std::fs::copy(&payload, &cached)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cached, std::fs::Permissions::from_mode(0o644))?;
    }

    let bin = tree.join("bin");
    fake_uname(&bin, "Linux", "x86_64")?;
    fake_ldd(&bin, "ldd (Ubuntu GLIBC 2.35-0ubuntu3.1) 2.35")?;
    fake_curl(&bin)?;
    std::fs::create_dir_all(tree.join("mise data"))?;
    std::fs::create_dir_all(tree.join("home"))?;
    let script = tree.join("tools/verify-pkl-artifact");
    let output = run_script(&script, &tree, 22)?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let passed = format!(
        "{} hashes to the committed linux-x64 checksum but cannot be executed",
        cached.display()
    );
    if output.status.code() != Some(1) {
        return Err(format!(
            "a checksum-matching but non-executable cache entry must not succeed; \
             status: {}; stderr: {stderr}",
            output.status
        )
        .into());
    }
    for fact in [
        &passed,
        "passing it over",
        "cannot obtain the linux-x64 artifact",
    ] {
        if !stderr.contains(fact) {
            return Err(format!(
                "the run must name {fact} — the probe refusal and the fetch that \
                 followed it; stderr: {stderr}"
            )
            .into());
        }
    }
    if !stdout.trim().is_empty() {
        return Err(
            format!("no path may be printed for an unrunnable artifact; stdout: {stdout}").into(),
        );
    }
    drop(std::fs::remove_dir_all(&tree));
    drop(std::fs::remove_dir_all(&payload_tree));
    Ok(())
}
