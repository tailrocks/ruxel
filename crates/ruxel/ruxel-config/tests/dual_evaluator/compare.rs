//! The two evaluators' answers and the comparison the corpus runs: both
//! must refuse or accept every example alike and lower a valid one to
//! the same engine input, and the outcome must be the one the corpus
//! declares.

use serde_json::Value;
use std::path::Path;
use std::process::Command;

use super::Result;

/// What an evaluator answered: the value it lowered to, or the error it
/// refused the file with.
pub(super) struct Answer {
    pub(super) value: Option<Value>,
    pub(super) error: Option<String>,
}

impl Answer {
    pub(super) fn failed(&self) -> bool {
        self.value.is_none()
    }
}

/// The evaluation under pklr, which the engine uses.
pub(super) fn under_pklr(path: &Path, root_dir: &Path) -> Answer {
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
pub(super) fn under_pkl(pkl: &Path, modules: &Path, path: &Path, root_dir: &Path) -> Answer {
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
pub(super) fn without_fingerprints(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("schemaFingerprint");
        object.remove("languageFingerprint");
    }
    value
}

/// The text the error must contain: `// expect-pklr:` when pklr's wording
/// differs from Pkl's, otherwise `// expect: `.
pub(super) fn expected(source: &str) -> Option<&str> {
    let line = |prefix: &str| source.lines().find_map(|line| line.strip_prefix(prefix));
    line("// expect-pklr: ").or_else(|| line("// expect: "))
}

/// The text the official CLI's error must contain.
pub(super) fn expected_official(source: &str) -> Option<&str> {
    source
        .lines()
        .find_map(|line| line.strip_prefix("// expect-pkl: "))
}

/// One example: the two evaluators must agree on it, and it must be the
/// outcome the corpus says it is.
pub(super) fn compare(
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
