use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempRepository(PathBuf);

impl TempRepository {
    fn new() -> Result<Self, String> {
        for _ in 0..100 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("ruxel-git-check-{}-{sequence}", std::process::id()));
            match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!("git: create isolated check repository: {error}"));
                }
            }
        }
        Err("git: cannot create isolated check repository".into())
    }
}

impl Drop for TempRepository {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) fn resolve<F>(repo: &str, version: &str, git: &F) -> Result<String, String>
where
    F: Fn(&[&str], Option<&str>) -> Result<(String, bool), String>,
{
    let temp = TempRepository::new()?;
    let path = temp.0.to_str().ok_or("git: temporary path is not UTF-8")?;
    let (_, cloned) = git(
        &["clone", "--bare", "--no-hardlinks", "--", repo, path],
        None,
    )?;
    if !cloned {
        return Err("git: isolated check clone failed".into());
    }
    let (resolved, found) = git(
        &["rev-parse", "--verify", &format!("{version}^{{commit}}")],
        Some(path),
    )?;
    if !found {
        return Err(format!("git: cannot resolve version {version}"));
    }
    Ok(resolved)
}
