//! The catalogue's systemd examples under both evaluators: pklr, the
//! evaluator the binary serves, and the official `pkl` CLI.
//!
//! pklr skips several checks the official evaluator makes — a class's own
//! type constraints, the engine schema's unions — so a workflow that only
//! pklr accepts can still lower to something the engine refuses. These
//! examples are therefore checked twice: both evaluators must accept a
//! valid workflow and lower it to the same engine input, and both must
//! refuse an invalid one with the message the corpus names.
//!
//! The `pkl` CLI is required: the test fails loudly when it is missing,
//! rather than skipping, because a skipped test would silently unpin the
//! engine schema's `reloaded` state and the `SystemdTimer` class.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The examples of the catalogue's systemd block: a service that takes
/// reload triggers, a timer, and the ways of writing them that are
/// refused.
const EXAMPLES: &[(&str, bool)] = &[
    ("valid/service-reload.pkl", true),
    ("valid/systemd-timer.pkl", true),
    ("valid/systemd-timer-default-service.pkl", true),
    ("valid/systemd-timer-large-delay.pkl", true),
    ("valid/systemd-timer-delay-spellings.pkl", true),
    ("valid/systemd-timer-long-name.pkl", true),
    ("invalid/service-both-triggers.pkl", false),
    ("invalid/service-mixed-triggers.pkl", false),
    ("invalid/reload-stopped-service.pkl", false),
    ("invalid/service-timer-name.pkl", false),
    ("invalid/timer-service-name.pkl", false),
    ("invalid/timer-empty-calendar.pkl", false),
    ("invalid/timer-spaces-calendar.pkl", false),
    ("invalid/timer-tab-calendar.pkl", false),
    ("invalid/timer-multiline-calendar.pkl", false),
    ("invalid/timer-persistent-without-calendar.pkl", false),
    ("invalid/timer-name-too-long.pkl", false),
    ("invalid/timer-boot-sub-millisecond.pkl", false),
    ("invalid/timer-boot-fractional-millisecond.pkl", false),
    ("invalid/timer-boot-fractional-millisecond-large.pkl", false),
    ("invalid/timer-boot-fractional-millisecond-huge.pkl", false),
    ("invalid/timer-boot-half-nanosecond.pkl", false),
    ("invalid/timer-boot-negative.pkl", false),
    ("invalid/timer-file-inside.pkl", false),
    ("invalid/reload-missing-dependency.pkl", false),
];

fn root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow"
    ))
    .to_path_buf()
}

/// Cases import the shared inventory from the corpus root, so that is the
/// directory they may read.
fn options() -> ruxel_config::EvalOptions {
    ruxel_config::EvalOptions {
        root_dir: Some(root()),
        ..ruxel_config::EvalOptions::default()
    }
}

/// The official evaluator. `RUXEL_PKL` names it; otherwise it is looked up
/// in `PATH` and in the usual places a mise install leaves it.
fn pkl() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("RUXEL_PKL") {
        let path = PathBuf::from(path);
        if !path.is_file() {
            return Err(format!("RUXEL_PKL names no file: {}", path.display()).into());
        }
        return Ok(path);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            candidates.push(directory.join("pkl"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let installs = PathBuf::from(home).join(".local/share/mise/installs/pkl");
        if let Ok(entries) = std::fs::read_dir(installs) {
            for entry in entries.flatten() {
                candidates.push(entry.path().join("pkl"));
            }
        }
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/pkl"));
    candidates.push(PathBuf::from("/usr/local/bin/pkl"));
    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }
    Err(format!(
        "the official `pkl` evaluator is missing: install it or point RUXEL_PKL at it \
         (looked in {candidates:?})"
    )
    .into())
}

/// The served modules, written where the `pkl` CLI reads them.
struct Modules(PathBuf);

impl Modules {
    fn new() -> Result<Self> {
        // The two tests run in parallel, and the clock may hand two threads
        // the same nanosecond: a shared directory would then be removed by
        // one test while the other still reads it. A counter gives every
        // directory of this process its own.
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ruxel-dual-evaluator-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path)?;
        for (module, text) in ruxel_config::schema::exported_modules() {
            let file = path.join(module.trim_start_matches('/'));
            let parent = file
                .parent()
                .ok_or_else(|| format!("{module} has no parent directory"))?;
            std::fs::create_dir_all(parent)?;
            std::fs::write(&file, text)?;
        }
        Ok(Self(path))
    }
}

impl Drop for Modules {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

/// The message a file names: `// expect-pklr:` when pklr's wording differs
/// from Pkl's, otherwise `// expect:`.
fn expected(source: &str, evaluator: &str) -> Result<String> {
    let line = |prefix: &str| {
        source
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .map(str::to_owned)
    };
    line(&format!("// expect-{evaluator}: "))
        .or_else(|| line("// expect: "))
        .ok_or_else(|| format!("no expect-{evaluator} line").into())
}

/// Whether a value is the engine's default, which the two evaluators may
/// spell differently.
fn is_default(key: &str, value: &serde_json::Value) -> bool {
    match (key, value) {
        ("delaySeconds", serde_json::Value::Number(number)) => number.as_i64() == Some(5),
        ("ignoreErrors" | "noLog", serde_json::Value::Bool(flag)) => !flag,
        ("loopVar", serde_json::Value::String(text)) => text == "item",
        _ => false,
    }
}

/// Drops nulls, empty collections and the engine defaults, and sorts the
/// keys, so that the two evaluators' outputs can be compared.
fn normalize(value: &serde_json::Value) -> Result<String> {
    fn strip(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut kept: Vec<(String, serde_json::Value)> = map
                    .iter()
                    .filter(|(key, value)| {
                        !value.is_null()
                            && value.as_array() != Some(&Vec::new())
                            && value.as_object() != Some(&serde_json::Map::new())
                            && !is_default(key, value)
                            && key.as_str() != "schemaFingerprint"
                            && key.as_str() != "languageFingerprint"
                    })
                    .map(|(key, value)| (key.clone(), strip(value)))
                    .collect();
                kept.sort_by(|left, right| left.0.cmp(&right.0));
                serde_json::Value::Object(kept.into_iter().collect())
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(strip).collect())
            }
            other => other.clone(),
        }
    }
    let stripped = strip(value);
    let config = stripped.get("config").cloned().unwrap_or(stripped);
    Ok(serde_json::to_string_pretty(&config)?)
}

/// The official evaluator's answer: whether it accepted the file, and
/// everything it said.
fn pkl_evaluates(modules: &Modules, file: &Path) -> Result<(bool, String)> {
    let output = Command::new(pkl()?)
        .arg("eval")
        .arg("-f")
        .arg("json")
        .arg("--module-path")
        .arg(&modules.0)
        .arg(file)
        .output()?;
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok((output.status.success(), said))
}

/// A valid workflow lowers to the same engine input under both evaluators.
#[test]
fn valid_examples_lower_to_the_same_engine_input() -> Result {
    let modules = Modules::new()?;
    for (name, _) in EXAMPLES.iter().filter(|(_, valid)| *valid) {
        let file = root().join(name);
        let pklr = ruxel_config::eval_config(&file, &options())
            .map_err(|error| format!("{name}: pklr refused it: {error}"))?;
        let (accepted, said) = pkl_evaluates(&modules, &file)?;
        assert!(accepted, "{name}: pkl refused it: {said}");
        let pkl: serde_json::Value = serde_json::from_str(&said)
            .map_err(|error| format!("{name}: pkl's output is not JSON: {error}"))?;
        assert_eq!(
            normalize(&pklr)?,
            normalize(&pkl)?,
            "{name}: the two evaluators lower it differently"
        );
    }
    Ok(())
}

/// An invalid workflow is refused by both evaluators, each with the
/// message its own wording names.
#[test]
fn invalid_examples_are_refused_by_both_evaluators() -> Result {
    let modules = Modules::new()?;
    for (name, _) in EXAMPLES.iter().filter(|(_, valid)| !*valid) {
        let file = root().join(name);
        let source = std::fs::read_to_string(&file)?;
        let pklr = ruxel_config::eval_config(&file, &options())
            .err()
            .ok_or_else(|| format!("{name}: pklr accepted it"))?
            .to_string();
        let expected_pklr = expected(&source, "pklr")?;
        assert!(
            pklr.contains(&expected_pklr),
            "{name}: pklr said {pklr:?}, not {expected_pklr:?}"
        );
        let (accepted, said) = pkl_evaluates(&modules, &file)?;
        assert!(!accepted, "{name}: pkl accepted it: {said}");
        let expected_pkl = expected(&source, "pkl")?;
        assert!(
            said.contains(&expected_pkl),
            "{name}: pkl said {said:?}, not {expected_pkl:?}"
        );
    }
    Ok(())
}
