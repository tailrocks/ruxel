use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
const PINS: &str = include_str!("../pins.txt");
fn collect(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let kind = item.file_type()?;
        let path = item.path();
        ensure!(!kind.is_symlink(), "symlink in pinned workloads");
        if kind.is_dir() {
            collect(root, &path, files)?;
        } else {
            ensure!(kind.is_file(), "non-file in pinned workloads");
            files.insert(path.strip_prefix(root)?.to_owned(), fs::read(path)?);
        }
    }
    Ok(())
}
pub fn verified(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut files = BTreeMap::new();
    collect(root, &root.join("testdata/workloads/pr66"), &mut files)?;
    let expected: BTreeMap<PathBuf, &str> = PINS
        .lines()
        .map(|line| {
            let (hash, name) = line.split_once("  ").context("invalid pin")?;
            Ok((PathBuf::from(name), hash))
        })
        .collect::<Result<_>>()?;
    ensure!(
        files.len() == expected.len(),
        "workload file count differs from pins"
    );
    for (name, bytes) in &files {
        let expected = expected.get(name).context("unpinned workload")?;
        let digest: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        ensure!(
            digest == *expected,
            "workload pin mismatch: {}",
            name.display()
        );
    }
    Ok(files)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modified_missing_and_extra_fixtures_fail() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .context("tools")?
            .parent()
            .context("root")?;
        let originals = verified(root)?;
        let tmp = tempfile::tempdir()?;
        for (name, bytes) in &originals {
            let target = tmp.path().join(name);
            fs::create_dir_all(target.parent().context("parent")?)?;
            fs::write(target, bytes)?;
        }
        assert!(verified(tmp.path()).is_ok());
        let victim = tmp.path().join("testdata/workloads/pr66/multihost.pkl");
        fs::write(&victim, "tampered")?;
        assert!(verified(tmp.path()).is_err());
        fs::write(
            &victim,
            &originals[Path::new("testdata/workloads/pr66/multihost.pkl")],
        )?;
        fs::remove_file(&victim)?;
        assert!(verified(tmp.path()).is_err());
        fs::write(
            &victim,
            &originals[Path::new("testdata/workloads/pr66/multihost.pkl")],
        )?;
        fs::write(tmp.path().join("testdata/workloads/pr66/extra"), "extra")?;
        assert!(verified(tmp.path()).is_err());
        Ok(())
    }
}
