//! `tools/verify-pkl-artifact` at the script level: the verifier
//! provisions the pkl artifact from the lockfile's own url and checksum,
//! so it must fetch exactly the asset the `[tools.pkl."platforms.…"]`
//! slot of the host platform pins — the alpine one on a musl host, the
//! glibc one on a glibc host that merely has a musl loader installed
//! beside the active libc — and it must refuse a platform upstream
//! publishes no artifact for with an unsupported-platform result of its
//! own: macOS `x86_64`, whose artifact vanished after 0.32.2, and
//! linux-arm64-musl, which never had one (apple/pkl 0.33.0 ships no
//! alpine aarch64 build).
//!
//! The committed wiring these verdicts depend on — the mise.lock slots
//! and the generated-check input lists — is asserted in the sibling
//! `verify_pkl_artifact_wiring.rs`.
//!
//! Each test runs the script itself in a child process against a fresh
//! scratch tree, the way the dual-evaluator discovery is covered
//! (`crates/ruxel/ruxel-config/tests/dual_evaluator.rs`): a script's
//! behavior is its output and its exit status. The fetch is stubbed —
//! a fake `curl` on the `PATH` records the url it was handed and lays
//! down a payload that is not the official binary — so a verifier that
//! reaches the fetch answers with the checksum refusal, the observable
//! proof that the committed asset was obtained and judged.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "verify_pkl_artifact/platforms.rs"]
mod platforms;
#[path = "verify_pkl_artifact/probe.rs"]
mod probe;

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
/// verifier scans. It is not the official binary, so a verifier that
/// reaches it passes it over on its checksum and fetches the committed
/// asset instead.
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

/// A fake `curl` in `bin` standing in for the fetch: it records the
/// arguments it was handed into the file `RUXEL_CURL_LOG` names, writes
/// a payload that is not the official binary to the `-o` destination it
/// finds among them, and exits with `RUXEL_CURL_STATUS`. The recording
/// is the observable that shows which committed url the verifier chose;
/// the payload is what turns a completed fetch into the checksum
/// refusal without any network.
fn fake_curl(bin: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(bin)?;
    let curl = bin.join("curl");
    std::fs::write(
        &curl,
        "#!/bin/sh\n\
         log=${RUXEL_CURL_LOG:-/dev/null}\n\
         printf '%s\\n' \"$*\" >>\"$log\"\n\
         out=\n\
         while [ $# -gt 0 ]; do\n\
           if [ \"$1\" = -o ]; then out=$2; shift 2; continue; fi\n\
           shift\n\
         done\n\
         if [ -n \"$out\" ]; then printf 'not the official pkl binary\\n' >\"$out\"; fi\n\
         exit ${RUXEL_CURL_STATUS:-0}\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(curl)
}

/// Whether the fetch was reached at all: the stub writes one line of
/// arguments per call into its log.
fn curl_log(scratch: &Path) -> Result<String> {
    match std::fs::read_to_string(scratch.join("curl.log")) {
        Ok(log) => Ok(log),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

/// The verifier run against a scratch tree: a stubbed `PATH` (the fake
/// `uname`, `ldd` and `curl` the test planted), a cache directory the
/// test names — whitespace included, because a configured path is one
/// directory, not a word list — and Mise data directories under the
/// scratch tree alone, so nothing of the operator's host is scanned.
fn run_script(script: &Path, scratch: &Path, curl_status: u8) -> Result<std::process::Output> {
    let output = Command::new("sh")
        .arg(script)
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", scratch.join("bin").display()),
        )
        .env("RUXEL_PKL_CACHE_DIR", scratch.join("cache with space"))
        .env("RUXEL_CURL_LOG", scratch.join("curl.log"))
        .env("RUXEL_CURL_STATUS", curl_status.to_string())
        .env("MISE_DATA_DIR", scratch.join("mise data"))
        .env("HOME", scratch.join("home"))
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CACHE_HOME")
        .output()?;
    Ok(output)
}

/// The checksum-refusal verdict: the committed asset was obtained and
/// does not hash to the committed checksum, so the run exits 1, names
/// the url it fetched, the checksum it wanted and the platform slot it
/// judged the host to need, and prints nothing on stdout — the absolute
/// path is a success verdict and belongs there alone.
fn assert_refused(
    output: &std::process::Output,
    url: &str,
    platform: &str,
    checksum: &str,
) -> Result {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut named = vec![
        format!("the artifact fetched from {url} is SHA-256 "),
        format!("for {platform}; refusing to run it"),
        checksum.to_string(),
    ];
    for fact in named.drain(..) {
        if !stderr.contains(&fact) {
            return Err(format!(
                "the refusal must name {fact}; status: {}; stderr: {stderr}",
                output.status
            )
            .into());
        }
    }
    if output.status.code() != Some(1) {
        return Err(format!(
            "a mismatched artifact must exit 1; status: {}; stderr: {stderr}",
            output.status
        )
        .into());
    }
    if !stdout.trim().is_empty() {
        return Err(format!("a refusal must print no path on stdout; stdout: {stdout}").into());
    }
    Ok(())
}

/// The verifier fetches exactly the asset the host platform's slot
/// pins. On a glibc linux-x64 host — the CI runner — that is
/// `pkl-linux-amd64`, and the payload the stub lays down is refused
/// against the committed checksum, after being written under the cache
/// directory the test named: one directory, whitespace included, never
/// a word list.
#[test]
fn the_verifier_fetches_the_committed_glibc_asset_for_linux_x64() -> Result {
    let tree = scratch("linux-x64-glibc")?;
    let bin = tree.join("bin");
    fake_uname(&bin, "Linux", "x86_64")?;
    fake_ldd(&bin, "ldd (Ubuntu GLIBC 2.35-0ubuntu3.1) 2.35")?;
    fake_curl(&bin)?;
    std::fs::create_dir_all(tree.join("home"))?;
    let output = run_script(&verifier(), &tree, 0)?;
    assert_refused(
        &output,
        "https://github.com/apple/pkl/releases/download/0.33.0/pkl-linux-amd64",
        "linux-x64",
        "b9a6ddea6b382b7d4db4cf78e77aa3858e0c8a7e02939d2ba298574b5d1fcc69",
    )?;
    let log = curl_log(&tree)?;
    if !log.contains("pkl-linux-amd64") || !log.contains("cache with space") {
        return Err(format!(
            "the fetch must take the committed linux-x64 url into the named cache \
             directory; curl was called with: {log}"
        )
        .into());
    }
    drop(std::fs::remove_dir_all(&tree));
    Ok(())
}

/// The musl slot must be reachable: on a host whose active libc is
/// musl, the fetch takes the hand-corrected alpine asset the
/// `platforms.linux-x64-musl` slot pins, never the glibc binary Aqua's
/// registry template would resolve there — the refusal names the
/// alpine url and the musl slot's committed checksum.
#[test]
fn the_verifier_fetches_the_committed_alpine_asset_for_a_musl_host() -> Result {
    let tree = scratch("linux-x64-musl")?;
    let bin = tree.join("bin");
    fake_uname(&bin, "Linux", "x86_64")?;
    fake_ldd(&bin, "musl libc (x86_64)")?;
    fake_curl(&bin)?;
    std::fs::create_dir_all(tree.join("home"))?;
    let output = run_script(&verifier(), &tree, 0)?;
    assert_refused(
        &output,
        "https://github.com/apple/pkl/releases/download/0.33.0/pkl-alpine-linux-amd64",
        "linux-x64-musl",
        "38e983b0953018888761c8b410ccdc6f27ce77407ad306e73e6369c07bc23d1f",
    )?;
    drop(std::fs::remove_dir_all(&tree));
    Ok(())
}

/// The musl slot must follow the ACTIVE libc. On a glibc host where
/// some toolchain left a musl loader installed, `ldd` still reports
/// glibc and the fetch must keep the glibc asset: the url handed to
/// the fetch is pkl-linux-amd64 and the refusal names linux-x64, never
/// the musl slot that the loader's mere presence used to select — a
/// musl verdict would reject the correct binary against the alpine
/// asset's checksum.
#[test]
fn the_verifier_follows_the_active_libc_over_an_installed_musl_loader() -> Result {
    let tree = scratch("active-libc-glibc")?;
    let bin = tree.join("bin");
    fake_uname(&bin, "Linux", "x86_64")?;
    fake_ldd(&bin, "ldd (Ubuntu GLIBC 2.35-0ubuntu3.1) 2.35")?;
    fake_curl(&bin)?;
    std::fs::create_dir_all(tree.join("home"))?;
    let output = run_script(&verifier(), &tree, 0)?;
    assert_refused(
        &output,
        "https://github.com/apple/pkl/releases/download/0.33.0/pkl-linux-amd64",
        "linux-x64",
        "b9a6ddea6b382b7d4db4cf78e77aa3858e0c8a7e02939d2ba298574b5d1fcc69",
    )?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("linux-x64-musl") {
        return Err(format!(
            "a glibc host must never be judged as linux-x64-musl; stderr: {stderr}"
        )
        .into());
    }
    drop(std::fs::remove_dir_all(&tree));
    Ok(())
}

/// A foreign Mise install never displaces the committed asset: a
/// binary already sitting at Mise's install path that does not hash to
/// the committed checksum is named and passed over, and the committed
/// asset is fetched anyway — an "already installed" wrong binary is
/// neither executed nor a verdict.
#[test]
fn the_verifier_passes_a_foreign_mise_install_over() -> Result {
    let tree = scratch("foreign-mise-install")?;
    let bin = tree.join("bin");
    fake_uname(&bin, "Linux", "x86_64")?;
    fake_ldd(&bin, "ldd (Ubuntu GLIBC 2.35-0ubuntu3.1) 2.35")?;
    fake_curl(&bin)?;
    let installed = fake_install(&tree.join("mise data"))?;
    std::fs::create_dir_all(tree.join("home"))?;
    let output = run_script(&verifier(), &tree, 0)?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let passed = format!(
        "{} does not match the committed linux-x64 checksum; passing it over",
        installed.display()
    );
    if !stderr.contains(&passed) {
        return Err(format!(
            "the verifier must name the foreign install it passed over; expected \
             {passed}; stderr: {stderr}"
        )
        .into());
    }
    if curl_log(&tree)?.trim().is_empty() {
        return Err(
            "a foreign install must not end the provisioning: the committed asset must still be fetched"
                .into(),
        );
    }
    assert_refused(
        &output,
        "https://github.com/apple/pkl/releases/download/0.33.0/pkl-linux-amd64",
        "linux-x64",
        "b9a6ddea6b382b7d4db4cf78e77aa3858e0c8a7e02939d2ba298574b5d1fcc69",
    )?;
    drop(std::fs::remove_dir_all(&tree));
    Ok(())
}

/// A missing artifact is its own verdict: nothing reusable under the
/// cache or Mise's directories and a failed fetch exit 1, naming the
/// url that could not be obtained and the checksum nothing matched —
/// distinct from exit 3, where no artifact exists to obtain at all.
#[test]
fn the_verifier_reports_an_unobtainable_artifact_as_missing() -> Result {
    let tree = scratch("unobtainable")?;
    let bin = tree.join("bin");
    fake_uname(&bin, "Linux", "x86_64")?;
    fake_ldd(&bin, "ldd (Ubuntu GLIBC 2.35-0ubuntu3.1) 2.35")?;
    fake_curl(&bin)?;
    std::fs::create_dir_all(tree.join("home"))?;
    let output = run_script(&verifier(), &tree, 22)?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let missing = "cannot obtain the linux-x64 artifact from \
                   https://github.com/apple/pkl/releases/download/0.33.0/pkl-linux-amd64";
    if output.status.code() != Some(1) || !stderr.contains(missing) {
        return Err(format!(
            "a failed fetch must exit 1 naming the url it could not obtain; status: {}; \
             stderr: {stderr}",
            output.status
        )
        .into());
    }
    if !stderr.contains("no cached artifact and no Mise install matches the committed checksum") {
        return Err(format!(
            "the missing verdict must say nothing matched the committed checksum; \
             stderr: {stderr}"
        )
        .into());
    }
    drop(std::fs::remove_dir_all(&tree));
    Ok(())
}
