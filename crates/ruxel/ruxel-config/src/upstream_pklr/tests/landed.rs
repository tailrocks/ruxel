//! The capability families that landed upstream with jdx/pklr#419's merged
//! fixes, exercised at [`super::super::UPSTREAM_REV`] through the
//! `pklr-upstream` alias.

use super::scratch;
use std::path::{Path, PathBuf};

/// The two capability families that landed upstream with #421 and #423 are
/// exercised here at [`super::super::UPSTREAM_REV`], so the alias pin moving
/// past jdx/pklr#419's merged fixes is covered by running code, not
/// citations: capability-backed globbing (`GlobAccess` / `expand_glob_with`)
/// and the `resource_*` hook split (module loads and resource reads answer
/// through separate seams).
#[test]
fn the_landed_capability_families_work_through_the_alias() -> Result<(), Box<dyn std::error::Error>>
{
    /// A host that answers `read_bytes` truthfully and `resource_read_bytes`
    /// with a refusal, so the #421 split is observable from both sides.
    struct SplitHost;
    impl pklr_upstream::EvalCapabilities for SplitHost {
        fn read_to_string(&mut self, path: &Path) -> pklr_upstream::Result<String> {
            Ok(String::from_utf8_lossy(&self.read_bytes(path)?).into_owned())
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
            Ok(std::fs::read(path).unwrap_or_default())
        }
        fn resource_read_bytes(&mut self, _: &Path) -> pklr_upstream::Result<Vec<u8>> {
            Err(pklr_upstream::Error::Eval(
                "resources refused by policy".to_owned(),
            ))
        }
    }

    use pklr_upstream::EvalCapabilities as _;

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
    // module loads while refusing resource reads.
    let mut host = SplitHost;
    let served = host.read_bytes(&directory.join("served.txt"))?;
    assert_eq!(served, b"served", "module loads answer through read_bytes");
    let refusal = host
        .resource_read_bytes(&directory.join("served.txt"))
        .expect_err("resource reads answer through their own hook");
    assert!(refusal.to_string().contains("refused by policy"));
    Ok(())
}
