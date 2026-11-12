//! The workflow examples of the secret language under both evaluators:
//! pklr, which the engine evaluates with, and the official `pkl` CLI, which
//! the language must also evaluate.
//!
//! The official CLI is required: where it is not installed, the check fails
//! loudly, because a skipped check passes every regression the corpus was
//! added to cover. `RUXEL_PKL` points the check at a binary of its own.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow"
    ))
    .to_path_buf()
}

/// The examples of the secret language: each must evaluate the same way
/// under both evaluators, and each invalid one must be refused by both.
const EXAMPLES: &[&str] = &[
    "valid/secret-op-reference-dotted.pkl",
    "valid/secret-literal-prefix.pkl",
    "invalid/secret-op-reference-dotted-selector.pkl",
    "invalid/secret-placeholder-orphaned-runtime.pkl",
    "invalid/secret-placeholder-orphaned-secret.pkl",
    "invalid/secret-unterminated-placeholder.pkl",
    "invalid/secret-decoder-oversized-prefix.pkl",
    "invalid/secret-placeholder-overlapping-malformed-prefix.pkl",
    "invalid/secret-placeholder-overlapping-malformed-underscore.pkl",
    "invalid/secret-placeholder-overlapping-malformed-unknown-tag.pkl",
    "invalid/secret-placeholder-overlapping-malformed-uppercase-tag.pkl",
    "invalid/secret-placeholder-overlapping-malformed-empty-body.pkl",
    "invalid/secret-placeholder-overlapping-malformed-missing-colon.pkl",
    "invalid/secret-placeholder-overlapping-malformed-punctuation.pkl",
    "invalid/secret-placeholder-overlapping-malformed-punctuation-tagless.pkl",
];

/// The official `pkl` binary.
///
/// `RUXEL_PKL` points at a binary of its own and is authoritative: a named binary that does not
/// run is a loud failure, never a reason to look further. Otherwise the binary is looked up on
/// the `PATH` and in the places an operator's tools live, because a test does not always carry
/// the shell's `PATH`. An absent binary is a loud failure too: the check is mandatory, and a
/// skipped check would pass every regression the corpus was added to cover.
fn pkl() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("RUXEL_PKL") {
        let path = PathBuf::from(path);
        return if runs(&path) {
            Ok(path)
        } else {
            Err(format!(
                "RUXEL_PKL points at {}, which is not a working official pkl",
                path.display()
            )
            .into())
        };
    }
    let mut candidates: Vec<PathBuf> = vec![PathBuf::from("pkl")];
    for data in mise_data_dirs() {
        let installs = data.join("installs/pkl");
        if let Ok(versions) = std::fs::read_dir(&installs) {
            let mut found: Vec<PathBuf> = versions
                .flatten()
                .map(|entry| entry.path().join("pkl"))
                .filter(|path| path.is_file())
                .collect();
            found.sort();
            candidates.extend(found.into_iter().rev());
        }
        candidates.push(data.join("shims/pkl"));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/pkl"));
    candidates.push(PathBuf::from("/usr/local/bin/pkl"));
    candidates
        .into_iter()
        .find(|candidate| runs(candidate))
        .ok_or_else(|| {
            "the official pkl is not installed: the dual-evaluator check is mandatory and \
             cannot run without it; install the Pkl CLI or point RUXEL_PKL at its binary"
                .into()
        })
}

/// Whether the binary answers `--version`: the probe that separates the
/// official CLI from an arbitrary file of the same name.
fn runs(candidate: &Path) -> bool {
    Command::new(candidate)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Every Mise data directory the lookup must scan, most specific first: `MISE_DATA_DIR` (else `XDG_DATA_HOME/mise`) and always the default under `$HOME`, where `mise install pkl@0.33.0` puts the binary.
fn mise_data_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(data) = std::env::var_os("MISE_DATA_DIR") {
        dirs.push(PathBuf::from(data));
    } else if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(xdg).join("mise"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/mise"));
    }
    dirs
}

/// The served modules as files, for the official CLI: it serves them under
/// `modulepath:/ruxel/`, as the binary does.
fn modules() -> Result<PathBuf> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "ruxel-dual-evaluator-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory)?;
    ruxel_config::schema::export(&directory)?;
    Ok(directory)
}

/// What an evaluator answered: the value it lowered to, or the error it
/// refused the file with.
struct Answer {
    value: Option<Value>,
    error: Option<String>,
}

impl Answer {
    fn failed(&self) -> bool {
        self.value.is_none()
    }
}

/// The evaluation under pklr, which the engine uses.
fn under_pklr(path: &Path, root_dir: &Path) -> Answer {
    let options = ruxel_config::EvalOptions {
        root_dir: Some(root_dir.to_path_buf()),
        ..ruxel_config::EvalOptions::default()
    };
    match ruxel_config::eval(path, &options) {
        Ok(value) => Answer {
            value: Some(value),
            error: None,
        },
        Err(error) => Answer {
            value: None,
            error: Some(error.to_string()),
        },
    }
}

/// The evaluation under the official CLI.
fn under_pkl(pkl: &Path, modules: &Path, path: &Path, root_dir: &Path) -> Answer {
    let output = Command::new(pkl)
        .args(["eval", "--format", "json"])
        .arg("--module-path")
        .arg(modules)
        .arg("--root-dir")
        .arg(root_dir)
        .arg(path)
        .output();
    let Ok(output) = output else {
        return Answer {
            value: None,
            error: Some("the official pkl could not run".to_owned()),
        };
    };
    if output.status.success() {
        match serde_json::from_slice::<Value>(&output.stdout) {
            Ok(value) => Answer {
                value: Some(value),
                error: None,
            },
            Err(error) => Answer {
                value: None,
                error: Some(error.to_string()),
            },
        }
    } else {
        Answer {
            value: None,
            error: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
        }
    }
}

/// The value without the fingerprints: the served modules carry a nonce,
/// the exported ones the hash alone, so the two evaluators never agree on
/// them.
fn without_fingerprints(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("schemaFingerprint");
        object.remove("languageFingerprint");
    }
    value
}

/// The text the error must contain: `// expect-pklr:` when pklr's wording
/// differs from Pkl's, otherwise `// expect: `.
fn expected(source: &str) -> Option<&str> {
    let line = |prefix: &str| source.lines().find_map(|line| line.strip_prefix(prefix));
    line("// expect-pklr: ").or_else(|| line("// expect: "))
}

/// The text the official CLI's error must contain.
fn expected_official(source: &str) -> Option<&str> {
    source
        .lines()
        .find_map(|line| line.strip_prefix("// expect-pkl: "))
}

/// One example: the two evaluators must agree on it, and it must be the
/// outcome the corpus says it is.
fn compare(
    name: &str,
    path: &Path,
    root_dir: &Path,
    pkl: &Path,
    modules: &Path,
    succeeds: bool,
) -> Result {
    let source = std::fs::read_to_string(path)?;
    let under_pklr = under_pklr(path, root_dir);
    let under_official = under_pkl(pkl, modules, path, root_dir);
    if under_pklr.failed() != under_official.failed() {
        return Err(format!(
            "{name}: pklr failed: {}, the official pkl failed: {}",
            under_pklr.error.unwrap_or_default(),
            under_official.error.unwrap_or_default()
        )
        .into());
    }
    if succeeds && under_pklr.failed() {
        return Err(format!(
            "{name}: pklr refused it: {}",
            under_pklr.error.unwrap_or_default()
        )
        .into());
    }
    // The inverse: an invalid example must fail under both evaluators, so
    // an example both accept identically can no longer silently pass.
    if !succeeds && !under_pklr.failed() {
        return Err(format!(
            "{name}: an invalid example must fail under both evaluators; \
             pklr and the official pkl both accepted it"
        )
        .into());
    }
    if under_pklr.failed() {
        let error = under_pklr.error.unwrap_or_default();
        let official = under_official.error.unwrap_or_default();
        if let Some(expected) = expected(&source)
            && !error.contains(expected)
        {
            return Err(format!("{name}: pklr refused it with {error:?}").into());
        }
        if let Some(expected) = expected_official(&source)
            && !official.contains(expected)
        {
            return Err(format!("{name}: the official pkl refused it with {official:?}").into());
        }
        return Ok(());
    }
    let value = without_fingerprints(&under_pklr.value.unwrap_or_default());
    let official = without_fingerprints(&under_official.value.unwrap_or_default());
    if value != official {
        return Err(format!("{name}: the two evaluators lowered it differently").into());
    }
    Ok(())
}

/// The examples of the secret language evaluate the same way under both
/// evaluators.
#[test]
fn the_examples_evaluate_the_same_way_under_both_evaluators() -> Result {
    let pkl = pkl()?;
    let modules = modules()?;
    for example in EXAMPLES {
        compare(
            example,
            &root().join(example),
            &root(),
            &pkl,
            &modules,
            example.starts_with("valid/"),
        )?;
    }
    Ok(())
}

/// A `join` separator of a million characters is a seven-digit length
/// prefix: the encoder writes it, so the decoder must read it. The
/// workflow is generated, because a million characters do not belong in a
/// committed file.
///
/// The secret is an environment value, not file content: the content of a
/// file is scanned for placeholders, and pklr's regular expressions do not
/// walk a million characters.
#[test]
fn a_long_join_separator_decodes_under_both_evaluators() -> Result {
    let pkl = pkl()?;
    let modules = modules()?;
    let directory = std::env::temp_dir().join(format!(
        "ruxel-dual-evaluator-long-{}-{}",
        std::process::id(),
        std::process::id()
    ));
    std::fs::create_dir_all(&directory)?;
    std::fs::copy(
        root().join("inventory.pkl"),
        directory.join("inventory.pkl"),
    )?;
    let separator = "x".repeat(1_000_000);
    let workflow = format!(
        "amends \"modulepath:/ruxel/Workflow.pkl\"\n\
         import \"modulepath:/ruxel/Core.pkl\" as core\n\
         import \"inventory.pkl\"\n\
         \n\
         stages {{ new Stage {{ title = \"t\"; hosts = inventory.workers; steps {{\n\
         \x20 new Command {{\n\
         \x20   run {{ \"/bin/true\" }}\n\
         \x20   environment {{ [\"BIG\"] = core.fnox(\"APP_SECRET\").join(\"{separator}\", core.fnox(\"APP_SECRET_TWO\")) }}\n\
         \x20 }}\n\
         }} }} }}\n"
    );
    let path = directory.join("long-separator.pkl");
    std::fs::write(&path, workflow)?;
    compare(
        "long-separator.pkl",
        &path,
        &directory,
        &pkl,
        &modules,
        true,
    )?;
    drop(std::fs::remove_dir_all(&directory));
    Ok(())
}

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
            "the_discovery_probe_reports_the_pkl_it_found",
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
