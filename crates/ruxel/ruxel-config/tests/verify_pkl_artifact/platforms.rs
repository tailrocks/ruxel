//! The unsupported-platform verdicts of `tools/verify-pkl-artifact`:
//! platforms upstream publishes no pkl artifact for answer with an
//! exit 3 of their own — never a missing, a tampered or a verified
//! verdict, and never a fetch.
//!
//! See the parent `verify_pkl_artifact.rs` for the harness.

use super::{fake_curl, fake_install, fake_ldd, fake_uname, run, scratch};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The unsupported-platform answer: exit 3 of its own, a message naming
/// the platform and the upstream facts, never a missing, a tampered or
/// a verified verdict — there is no official build to judge against —
/// and never a fetch: nothing is downloaded for a platform that has no
/// artifact to download.
fn assert_unsupported(
    output: &std::process::Output,
    scratch: &std::path::Path,
    platform: &str,
    facts: &[&str],
) -> Result {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.code() != Some(3) {
        return Err(format!(
            "the verifier must exit 3 (unsupported platform) on {platform}; status: {}; stderr: {stderr}",
            output.status
        )
        .into());
    }
    let mut names: Vec<&str> = vec![platform];
    names.extend_from_slice(facts);
    for fact in names {
        if !stderr.contains(fact) {
            return Err(format!(
                "the unsupported-platform message must name {fact}; stderr: {stderr}"
            )
            .into());
        }
    }
    for verdict in [
        "SHA-256",
        "refusing to run",
        "cannot obtain",
        "does not match the committed",
        "matches the committed",
        "carries no pkl url and checksum",
    ] {
        if stderr.contains(verdict) || stdout.contains(verdict) {
            return Err(format!(
                "{platform} must answer with the unsupported platform alone, never a tamper, a missing or a verified verdict ({verdict}); stdout: {stdout}; stderr: {stderr}"
            )
            .into());
        }
    }
    let log = super::curl_log(scratch)?;
    if !log.trim().is_empty() {
        return Err(format!(
            "{platform} must never fetch an artifact; curl was called with: {log}"
        )
        .into());
    }
    Ok(())
}

/// macOS `x86_64` has no official pkl artifact — upstream's last Intel
/// build was 0.32.2's `pkl-macos-amd64`, and 0.33.0 ships only
/// `pkl-macos-aarch64` — so the verifier must answer that platform
/// with an unsupported-platform result of its own: a distinct exit
/// code, a message naming the upstream fact, and no fetch.
///
/// The fake install is present, so a verifier that judged the platform
/// anyway would hash it and report a checksum mismatch, and one that
/// looked the lockfile up first would report the missing slot entry.
/// Neither is true: there is no official build to judge it against, so
/// the answer must name that instead.
#[test]
fn the_verifier_refuses_macos_x64_as_an_unsupported_platform() -> Result {
    let tree = scratch("macos-x64")?;
    let bin = tree.join("bin");
    fake_uname(&bin, "Darwin", "x86_64")?;
    fake_curl(&bin)?;
    let _installed = fake_install(&tree.join("mise data"))?;
    std::fs::create_dir_all(tree.join("home"))?;
    let output = run(&tree, 0)?;
    assert_unsupported(
        &output,
        &tree,
        "macos-x64",
        &["0.32.2", "pkl-macos-aarch64"],
    )?;
    drop(std::fs::remove_dir_all(&tree));
    Ok(())
}

/// linux-arm64-musl has no official pkl artifact: apple/pkl 0.33.0
/// ships no alpine aarch64 build (only pkl-alpine-linux-amd64), and
/// pkl-linux-aarch64 is a glibc binary that cannot start on a musl
/// host. The platform must answer with the unsupported-platform result
/// of its own — the checksum of a binary that cannot run is not a
/// verdict — never the checksum refusal against the glibc asset's
/// entry, and never a fetch.
#[test]
fn the_verifier_refuses_linux_arm64_musl_as_an_unsupported_platform() -> Result {
    let tree = scratch("linux-arm64-musl")?;
    let bin = tree.join("bin");
    fake_uname(&bin, "Linux", "aarch64")?;
    fake_ldd(&bin, "musl libc (aarch64)")?;
    fake_curl(&bin)?;
    let _installed = fake_install(&tree.join("mise data"))?;
    std::fs::create_dir_all(tree.join("home"))?;
    let output = run(&tree, 0)?;
    assert_unsupported(&output, &tree, "linux-arm64-musl", &["alpine", "glibc"])?;
    drop(std::fs::remove_dir_all(&tree));
    Ok(())
}
