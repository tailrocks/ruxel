//! The workflow examples of the secret language under both evaluators:
//! pklr, which the engine evaluates with, and the official `pkl` CLI, which
//! the language must also evaluate.
//!
//! The official CLI is required: where it is not installed, the check fails
//! loudly, because a skipped check passes every regression the corpus was
//! added to cover. `RUXEL_PKL` names the one binary the check executes:
//! the artifact `tools/verify-pkl-artifact` hashed and printed and the mise
//! `dual-evaluator` task exported, so verification and execution share one
//! resolution.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "dual_evaluator/chain.rs"]
mod chain;
#[path = "dual_evaluator/discovery.rs"]
mod discovery;
#[path = "dual_evaluator/join_separator.rs"]
mod join_separator;

#[path = "dual_evaluator/retired_path.rs"]
mod retired_path;

#[path = "dual_evaluator/corpus_files.rs"]
mod corpus_files;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow"
    ))
    .to_path_buf()
}

/// The examples both evaluators must lower the same way: the secret
/// language's, the user step that lowers to the engine's user operation,
/// the Linux catalogue's identity rules (an account's groups are a set)
/// and refusals (the root ids, the root group, a sysctl's file) —
/// including its type-constraint refusals, whose official-CLI wording a
/// `// expect-pkl:` line names — and the apt completions: a hold, a
/// system upgrade, a preference and a debconf selection lower the same
/// way under both evaluators, and the bad selector, the secret carried
/// as static text and the hold that conflicts with a pin are refused by
/// both, each with its own wording. Each invalid one must be refused by
/// both.
const EXAMPLES: &[&str] = &[
    "valid/user-missing-account.pkl",
    "valid/read-resource.pkl",
    "valid/secret-op-reference-dotted.pkl",
    "valid/secret-literal-prefix.pkl",
    "invalid/secret-op-reference-dotted-selector.pkl",
    "invalid/secret-placeholder-orphaned-runtime.pkl",
    "invalid/secret-placeholder-orphaned-secret.pkl",
    "invalid/secret-unterminated-placeholder.pkl",
    "invalid/secret-decoder-oversized-prefix.pkl",
    "valid/apt-hold.pkl",
    "valid/apt-hold-non-movements.pkl",
    "valid/apt-hold-equal-version.pkl",
    "valid/apt-hold-move-in-later-stage.pkl",
    "valid/apt-system-upgrade.pkl",
    "valid/apt-preference.pkl",
    "valid/apt-preference-collision.pkl",
    "valid/apt-preference-pin-forms.pkl",
    "valid/apt-preference-disjoint-selectors.pkl",
    "valid/debconf-entry.pkl",
    "invalid/apt-hold-conflicting-pin.pkl",
    "invalid/apt-hold-conflicting-latest.pkl",
    "invalid/apt-hold-moved-in-later-stage.pkl",
    "invalid/apt-hold-moved-across-aliases.pkl",
    "valid/apt-hold-superseded.pkl",
    "invalid/apt-hold-superseded-across-hosts.pkl",
    "invalid/apt-hold-superseded-partial-hosts.pkl",
    "invalid/apt-hold-unversioned-moved-by-pin.pkl",
    "invalid/apt-hold-unversioned-moved-by-pin-later-stage.pkl",
    "invalid/apt-hold-revision-moved-in-later-stage.pkl",
    "invalid/apt-hold-with-system-upgrade.pkl",
    "invalid/apt-hold-with-system-upgrade-later-stage.pkl",
    "invalid/apt-preference-package-selector-pin.pkl",
    "invalid/apt-preference-pseudo-arch.pkl",
    "invalid/apt-preference-priority-mismatch.pkl",
    "invalid/apt-preference-priority-zero.pkl",
    "invalid/apt-preference-pin-blank.pkl",
    "invalid/apt-preference-pin-release-empty.pkl",
    "invalid/apt-preference-pin-release-unknown.pkl",
    "invalid/apt-preference-pin-source-version.pkl",
    "invalid/apt-preference-pin-type.pkl",
    "invalid/apt-preference-pin-version-whitespace.pkl",
    "invalid/apt-preference-overlapping-selectors.pkl",
    "invalid/apt-preference-overlapping-selectors-later-stage.pkl",
    "invalid/apt-preference-file-at-fragment.pkl",
    "invalid/apt-preference-fragment-child.pkl",
    "invalid/apt-preference-before-latest.pkl",
    "invalid/apt-preference-before-latest-later-stage.pkl",
    "valid/apt-preference-different-hosts.pkl",
    "invalid/apt-preference-before-install.pkl",
    "invalid/apt-preference-before-system-upgrade.pkl",
    "invalid/apt-index-before-system-upgrade.pkl",
    "invalid/apt-index-before-system-upgrade-later-stage.pkl",
    "invalid/engine-apt-hold-latest.pkl",
    "invalid/engine-debconf-package-glob.pkl",
    "invalid/engine-debconf-value-multiline.pkl",
    "invalid/engine-debconf-question-space.pkl",
    "invalid/apt-repository-before-install.pkl",
    "invalid/apt-repository-before-install-later-stage.pkl",
    "invalid/apt-repository-before-system-upgrade.pkl",
    "invalid/apt-repository-before-system-upgrade-later-stage.pkl",
    "invalid/apt-preference-priority-range.pkl",
    "invalid/apt-pin-with-system-upgrade.pkl",
    "invalid/apt-system-upgrade-with-pin.pkl",
    "invalid/apt-preference-overlapping-globs.pkl",
    "invalid/apt-preference-overlapping-arches.pkl",
    "invalid/debconf-entry-pseudo-arch.pkl",
    "invalid/debconf-entry-bad-type.pkl",
    "invalid/debconf-entry-secret-as-static.pkl",
    "invalid/debconf-entry-secret-collision.pkl",
    "invalid/debconf-entry-shared-question.pkl",
    "invalid/debconf-entry-question-space.pkl",
    "invalid/debconf-entry-type-text.pkl",
    "invalid/debconf-entry-password-static.pkl",
    "invalid/debconf-entry-boolean-literal.pkl",
    "invalid/debconf-entry-package-glob.pkl",
    "invalid/debconf-value-multiline.pkl",
    "invalid/secret-placeholder-overlapping-malformed-prefix.pkl",
    "invalid/secret-placeholder-overlapping-malformed-underscore.pkl",
    "invalid/secret-placeholder-overlapping-malformed-unknown-tag.pkl",
    "invalid/secret-placeholder-overlapping-malformed-uppercase-tag.pkl",
    "invalid/secret-placeholder-overlapping-malformed-chain.pkl",
    "invalid/secret-placeholder-overlapping-malformed-empty-body.pkl",
    "invalid/secret-placeholder-overlapping-malformed-missing-colon.pkl",
    "invalid/secret-placeholder-overlapping-malformed-punctuation.pkl",
    "invalid/secret-placeholder-overlapping-malformed-punctuation-tagless.pkl",
    "invalid/secret-placeholder-overlapping-malformed-whitespace.pkl",
    "invalid/secret-placeholder-overlapping-malformed-newline.pkl",
    "invalid/secret-placeholder-overlapping-runtime-prefix.pkl",
    "invalid/secret-placeholder-overlapping-runtime-prefix-encoding.pkl",
    "invalid/secret-placeholder-overlapping-unterminated.pkl",
    "invalid/secret-placeholder-overlapping-unterminated-runtime.pkl",
    "invalid/secret-placeholder-overlapping-unterminated-body.pkl",
    "valid/secret-placeholder-shared-terminator.pkl",
    "valid/linux-user-groups-order.pkl",
    "valid/linux-sysctl.pkl",
    "valid/linux-user-watch.pkl",
    "valid/linux-user-groups-empty.pkl",
    "invalid/linux-user-groups-unset-vs-empty.pkl",
    "invalid/linux-user-shell-root.pkl",
    "invalid/linux-user-home-root.pkl",
    "invalid/linux-group-gid-zero.pkl",
    "invalid/linux-group-absent-root.pkl",
    "invalid/linux-sysctl-file-clash.pkl",
    "invalid/linux-file-sysctl-clash.pkl",
    "invalid/linux-group-name.pkl",
    "invalid/linux-login-shell-user-colon.pkl",
    "invalid/linux-login-shell-user-nul.pkl",
    "invalid/linux-sysctl-value-newline.pkl",
    "invalid/linux-user-groups-invalid-name.pkl",
    "invalid/linux-user-shell-not-absolute.pkl",
    "invalid/linux-user-uid-out-of-range.pkl",
    "invalid/linux-user-root-uid.pkl",
    "invalid/linux-user-root-group.pkl",
    "invalid/linux-group-root-gid.pkl",
    "invalid/linux-sysctl-path-root.pkl",
    "invalid/linux-sysctl-clash-apt-repository.pkl",
    "invalid/linux-file-above-sysctl.pkl",
    "invalid/linux-directory-sysctl-clash.pkl",
    "invalid/linux-user-home-conflict.pkl",
    "invalid/linux-user-group-absent.pkl",
    "invalid/linux-group-absent-after-user.pkl",
    "invalid/linux-user-needs-later-group.pkl",
];

/// The official `pkl` binary.
///
/// `RUXEL_PKL` is the whole resolution: the absolute path `tools/verify-pkl-artifact` hashed
/// against the committed checksum and printed, which the mise `dual-evaluator` task exports to
/// this suite. There is no fallback to look the binary up on the `PATH` or in the places an
/// operator's tools live: verification and execution must resolve to one binary, so a `pkl` the
/// verifier never hashed may never run, and where no verified path is supplied the suite fails
/// loudly instead of picking one of its own. A named binary that does not run is a loud failure
/// too: the check is mandatory, and a skipped check would pass every regression the corpus was
/// added to cover.
fn pkl() -> Result<PathBuf> {
    let Some(path) = std::env::var_os("RUXEL_PKL") else {
        return Err(
            "no verified official pkl was supplied: RUXEL_PKL must name the artifact that \
             tools/verify-pkl-artifact hashed and printed; the dual-evaluator check is \
             mandatory and executes exactly the verified binary, never one a PATH lookup \
             happens to find"
                .into(),
        );
    };
    let path = PathBuf::from(path);
    if runs(&path) {
        Ok(path)
    } else {
        Err(format!(
            "RUXEL_PKL points at {}, which is not a working official pkl",
            path.display()
        )
        .into())
    }
}

/// Whether the binary answers `--version`: the probe that separates the
/// official CLI from an arbitrary file of the same name.
fn runs(candidate: &Path) -> bool {
    Command::new(candidate)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
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
