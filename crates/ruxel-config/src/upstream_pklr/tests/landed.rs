//! The capability families that landed upstream with jdx/pklr#419's merged
//! fixes, exercised at [`super::super::UPSTREAM_REV`] through the
//! `pklr-upstream` alias.

use super::scratch;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The hooks the evaluator asked the host for, in the order it asked.
type Calls = Arc<Mutex<Vec<String>>>;

/// A host that answers module reads truthfully and resource reads with a
/// refusal, so the #421 split is observable from both sides.
struct SplitHost {
    /// The hooks the evaluator called.
    calls: Calls,
}

impl SplitHost {
    /// Record the hook the evaluator asked for, and the path it asked about.
    fn record(&self, hook: &str, path: &Path) {
        self.calls
            .lock()
            .expect("the call log")
            .push(format!("{hook}:{}", path.display()));
    }
}

impl pklr_upstream::EvalCapabilities for SplitHost {
    fn read_to_string(&mut self, path: &Path) -> pklr_upstream::Result<String> {
        self.record("read_to_string", path);
        Ok(std::fs::read_to_string(path).unwrap_or_default())
    }

    fn path_exists(&mut self, path: &Path) -> pklr_upstream::Result<bool> {
        Ok(path.exists())
    }

    fn canonicalize(&mut self, path: &Path) -> pklr_upstream::Result<PathBuf> {
        Ok(path.to_path_buf())
    }

    fn read_env(&mut self, _: &str) -> pklr_upstream::Result<Option<String>> {
        Ok(None)
    }

    fn fetch_text(&mut self, url: &str) -> pklr_upstream::Result<String> {
        Err(pklr_upstream::Error::Eval(format!(
            "no network in this test: {url}"
        )))
    }

    fn fetch_bytes(&mut self, url: &str) -> pklr_upstream::Result<Vec<u8>> {
        Err(pklr_upstream::Error::Eval(format!(
            "no network in this test: {url}"
        )))
    }

    fn temp_dir(&mut self, _: &str) -> pklr_upstream::Result<PathBuf> {
        Ok(std::env::temp_dir())
    }

    fn read_bytes(&mut self, path: &Path) -> pklr_upstream::Result<Vec<u8>> {
        self.record("read_bytes", path);
        Ok(std::fs::read(path).unwrap_or_default())
    }

    fn resource_read_bytes(&mut self, path: &Path) -> pklr_upstream::Result<Vec<u8>> {
        self.record("resource_read_bytes", path);
        Err(pklr_upstream::Error::Eval(
            "resources refused by policy".to_owned(),
        ))
    }
}

/// The two capability families that landed upstream with #421 and #423 are
/// exercised here at [`super::super::UPSTREAM_REV`], so the alias pin moving
/// past jdx/pklr#419's merged fixes is covered by running code, not
/// citations: capability-backed globbing (`GlobAccess` / `expand_glob_with`)
/// and the `resource_*` hook split (module loads and resource reads answer
/// through separate seams).
#[test]
fn the_landed_capability_families_work_through_the_alias() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = scratch("landed");
    std::fs::write(directory.join("served.txt"), "served")?;

    // #423: glob expansion over a host-provided file system.
    let expanded =
        pklr_upstream::expand_glob_with(&directory, "*.txt", &mut pklr_upstream::NativeGlobAccess)?;
    assert!(
        expanded.iter().any(|path| path.ends_with("served.txt")),
        "{expanded:?}"
    );

    // #421: the resource hooks are a seam of their own, so a host can allow
    // module loads while refusing resource reads. The split is observed
    // through upstream's evaluator, not through a direct call into this
    // test's own trait methods: only the evaluator can route the read to
    // the hook upstream chose for it, so a regression that routes
    // `read("file:…")` through `read_bytes` is caught here.
    let resource = directory.join("served.txt");
    let module = directory.join("main.pkl");
    std::fs::write(
        &module,
        format!("value = read(\"file:{}\")\n", resource.display()),
    )?;
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let mut evaluator = pklr_upstream::Evaluator::with_capabilities(SplitHost {
        calls: calls.clone(),
    });
    let refusal = evaluator
        .eval_file(&module)
        .expect_err("the resource read is refused by the resource hook");
    assert!(
        refusal.to_string().contains("refused by policy"),
        "{refusal}"
    );

    let calls = calls.lock().expect("the call log");
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("resource_read_bytes:") && call.ends_with("served.txt")),
        "the evaluator did not route the read through the resource hook: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("read_bytes:") && call.ends_with("served.txt")),
        "the evaluator routed the resource read through the module hook: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("read_to_string:") && call.ends_with("main.pkl")),
        "the evaluator did not load the module through the module hook: {calls:?}"
    );
    Ok(())
}
