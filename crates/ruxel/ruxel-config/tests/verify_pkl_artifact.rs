//! `tools/verify-pkl-artifact` at the script level: the verifier must
//! reach the binary under every Mise data directory it names, whatever
//! characters the path carries, and it must refuse a platform upstream
//! publishes no artifact for with an unsupported-platform result of its
//! own — macOS `x86_64`, whose artifact vanished after 0.32.2, and
//! linux-arm64-musl, which never had one (apple/pkl 0.33.0 ships no
//! alpine aarch64 build). The musl slot must follow the active libc,
//! never a musl loader that some toolchain merely installed beside it.
//!
//! The committed wiring these verdicts depend on — the mise.lock slots
//! and the generated-check input lists — is asserted in the sibling
//! `verify_pkl_artifact_wiring.rs`.
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
/// binary answers "no pkl 0.33.0 install found" instead. The refusal
/// carries exit 1 — the code of a missing or a mismatched binary, and
/// the code the unsupported-platform answer must differ from.
fn assert_reached(output: &std::process::Output, installed: &Path) -> Result {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reached = format!("{} is SHA-256 ", installed.display());
    if output.status.code() != Some(1) || !stderr.contains(&*reached) {
        return Err(format!(
            "the verifier must reach the fake at {} and refuse it on its checksum with exit 1; status: {}; stderr: {stderr}",
            installed.display(),
            output.status
        )
        .into());
    }
    Ok(())
}

/// Like [`assert_reached`], and the refusal must name the platform slot
/// whose committed checksum the binary was compared against: the named
/// slot is the observable that shows which libc's asset the verifier
/// judged the host to need.
fn assert_reached_as(output: &std::process::Output, installed: &Path, platform: &str) -> Result {
    assert_reached(output, installed)?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let named = format!("for {platform}; refusing to run it");
    if !stderr.contains(&*named) {
        return Err(format!(
            "the refusal must name the committed checksum of {platform}; stderr: {stderr}"
        )
        .into());
    }
    Ok(())
}

/// The unsupported-platform answer: exit 3 of its own, a message naming
/// the platform and the upstream facts, and never a missing, a tampered
/// or a verified verdict — there is no official build to judge against.
fn assert_unsupported(output: &std::process::Output, platform: &str, facts: &[&str]) -> Result {
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
        "no pkl 0.33.0 install found",
        "carries no pkl checksum",
        "matches the committed",
    ] {
        if stderr.contains(verdict) || stdout.contains(verdict) {
            return Err(format!(
                "{platform} must answer with the unsupported platform alone, never a tamper, a missing or a verified verdict ({verdict}); stdout: {stdout}; stderr: {stderr}"
            )
            .into());
        }
    }
    Ok(())
}

/// A fake `uname` in `bin` that reports the given host. The verifier
/// asks `uname` for the operating system and the architecture and
/// nothing else about the host reaches it, so this is how a test selects
/// a platform from any host.
fn fake_uname(bin: &Path, os: &str, arch: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(bin)?;
    let uname = bin.join("uname");
    std::fs::write(
        &uname,
        format!("#!/bin/sh\ncase \"${{1:-}}\" in\n  -m) echo {arch} ;;\n  *) echo {os} ;;\nesac\n"),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&uname, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(uname)
}

/// A fake `ldd` in `bin` that answers every probe with `banner`: the
/// voice of the active libc, which is the only libc signal the verifier
/// may trust.
fn fake_ldd(bin: &Path, banner: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(bin)?;
    let ldd = bin.join("ldd");
    std::fs::write(&ldd, format!("#!/bin/sh\ncat <<'EOF'\n{banner}\nEOF\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ldd, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(ldd)
}

/// A fake `ls` in `bin` that answers every probe with `code`, standing
/// in for a host where a musl loader is (or is not) installed beside
/// the active libc.
fn fake_ls(bin: &Path, code: i32) -> Result<PathBuf> {
    std::fs::create_dir_all(bin)?;
    let ls = bin.join("ls");
    std::fs::write(&ls, format!("#!/bin/sh\nexit {code}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ls, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(ls)
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

/// macOS `x86_64` has no official pkl artifact — upstream's last Intel
/// build was 0.32.2's `pkl-macos-amd64`, and 0.33.0 ships only
/// `pkl-macos-aarch64` — so the verifier must answer that platform
/// with an unsupported-platform result of its own: a distinct exit
/// code and a message naming the upstream fact.
///
/// The fake install is present, so a verifier that judged the platform
/// anyway would hash it and report a checksum mismatch, and one that
/// looked the checksum up first would report the missing checksum
/// entry. Neither is true: there is no official build to judge it
/// against, so the answer must name that instead.
#[test]
fn the_verifier_refuses_macos_x64_as_an_unsupported_platform() -> Result {
    let scratch = scratch("macos-x64")?;
    let bin = scratch.join("bin");
    fake_uname(&bin, "Darwin", "x86_64")?;
    let data = scratch.join("mise data");
    let _installed = fake_install(&data)?;
    let home = scratch.join("home");
    std::fs::create_dir_all(&home)?;
    let output = Command::new("/bin/sh")
        .arg(verifier())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("MISE_DATA_DIR", &data)
        .env("HOME", &home)
        .env_remove("XDG_DATA_HOME")
        .output()?;
    assert_unsupported(&output, "macos-x64", &["0.32.2", "pkl-macos-aarch64"])?;
    drop(std::fs::remove_dir_all(&scratch));
    Ok(())
}

/// linux-arm64-musl has no official pkl artifact: apple/pkl 0.33.0
/// ships no alpine aarch64 build (only pkl-alpine-linux-amd64), and
/// pkl-linux-aarch64 is a glibc binary that cannot start on a musl
/// host. The platform must answer with the unsupported-platform result
/// of its own — the checksum of a binary that cannot run is not a
/// verdict — never the checksum refusal against the glibc asset's
/// entry.
#[test]
fn the_verifier_refuses_linux_arm64_musl_as_an_unsupported_platform() -> Result {
    let scratch = scratch("linux-arm64-musl")?;
    let bin = scratch.join("bin");
    fake_uname(&bin, "Linux", "aarch64")?;
    fake_ldd(&bin, "musl libc (aarch64)")?;
    let data = scratch.join("mise data");
    let _installed = fake_install(&data)?;
    let home = scratch.join("home");
    std::fs::create_dir_all(&home)?;
    let output = Command::new("/bin/sh")
        .arg(verifier())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("MISE_DATA_DIR", &data)
        .env("HOME", &home)
        .env_remove("XDG_DATA_HOME")
        .output()?;
    assert_unsupported(&output, "linux-arm64-musl", &["alpine", "glibc"])?;
    drop(std::fs::remove_dir_all(&scratch));
    Ok(())
}

/// The musl slot must follow the ACTIVE libc. On a glibc host where
/// some toolchain left a musl loader installed, `ldd` still reports
/// glibc and the verifier must keep the glibc slot: the refusal names
/// linux-x64, never the musl slot that the loader's mere presence used
/// to select — a musl verdict would reject the correct binary against
/// the alpine asset's checksum.
#[test]
fn the_verifier_follows_the_active_libc_over_an_installed_musl_loader() -> Result {
    let scratch = scratch("active-libc-glibc")?;
    let bin = scratch.join("bin");
    fake_uname(&bin, "Linux", "x86_64")?;
    fake_ldd(&bin, "ldd (Ubuntu GLIBC 2.35-0ubuntu3.1) 2.35")?;
    fake_ls(&bin, 0)?;
    let data = scratch.join("mise data");
    let installed = fake_install(&data)?;
    let home = scratch.join("home");
    std::fs::create_dir_all(&home)?;
    let output = Command::new("/bin/sh")
        .arg(verifier())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("MISE_DATA_DIR", &data)
        .env("HOME", &home)
        .env_remove("XDG_DATA_HOME")
        .output()?;
    assert_reached_as(&output, &installed, "linux-x64")?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("linux-x64-musl") {
        return Err(format!(
            "a glibc host must never be judged as linux-x64-musl; stderr: {stderr}"
        )
        .into());
    }
    drop(std::fs::remove_dir_all(&scratch));
    Ok(())
}

/// And when the active libc is musl, the musl slot is selected on the
/// `ldd` word alone: no musl loader file needs to be installed beside
/// it. The refusal names linux-x64-musl, the alpine asset's checksum.
#[test]
fn the_verifier_selects_the_musl_slot_when_the_active_libc_is_musl() -> Result {
    let scratch = scratch("active-libc-musl")?;
    let bin = scratch.join("bin");
    fake_uname(&bin, "Linux", "x86_64")?;
    fake_ldd(&bin, "musl libc (x86_64)")?;
    fake_ls(&bin, 2)?;
    let data = scratch.join("mise data");
    let installed = fake_install(&data)?;
    let home = scratch.join("home");
    std::fs::create_dir_all(&home)?;
    let output = Command::new("/bin/sh")
        .arg(verifier())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("MISE_DATA_DIR", &data)
        .env("HOME", &home)
        .env_remove("XDG_DATA_HOME")
        .output()?;
    assert_reached_as(&output, &installed, "linux-x64-musl")?;
    drop(std::fs::remove_dir_all(&scratch));
    Ok(())
}
