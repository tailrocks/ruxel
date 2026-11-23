//! `ruxel schema export`: the served modules as files, for editors and the
//! `pkl` CLI.

use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::Path;

/// Write every module below `dir` (for example `dir/ruxel/Workflow.pkl`).
///
/// The tree is built in a fresh staging directory inside `dir` through
/// directory descriptors, with every open refusing symlinks, so nothing
/// planted or swapped in under `dir` can redirect a write. The new tree then
/// replaces the old one as a whole: the old tree is moved aside and the new
/// one moved into its place. A reader, or another export racing this one,
/// sees the old tree or a complete new one, never a mix; if another export
/// puts its tree in place first, this one fails instead.
///
/// # Errors
/// Returns an IO error when the tree cannot be written or put in place; the
/// previous export is then left as it was.
pub fn export(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let base = rustix::fs::open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let token = unique();
    let staging = format!(".ruxel-export.{token}");
    let previous = format!(".ruxel-previous.{token}");
    rustix::fs::mkdirat(&base, staging.as_str(), Mode::from_raw_mode(0o700))?;
    let result = build(&base, &staging).and_then(|root| swap(&base, &staging, &root, &previous));
    // `remove_dir_all` removes a symlink itself rather than following it.
    drop(std::fs::remove_dir_all(dir.join(&staging)));
    drop(std::fs::remove_dir_all(dir.join(&previous)));
    result
}

/// Write the modules below `base/staging` and return their common top
/// directory (`ruxel`).
fn build(base: &OwnedFd, staging: &str) -> std::io::Result<String> {
    let stage = open_directory(base, staging)?;
    let mut root: Option<String> = None;
    for (path, text) in super::exported_modules() {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let Some((name, directories)) = parts.split_last() else {
            return Err(std::io::Error::other(format!("empty module path {path}")));
        };
        let top = directories.first().copied().unwrap_or(name);
        match &root {
            Some(root) if root != top => {
                return Err(std::io::Error::other(format!(
                    "module {path} is outside {root}/"
                )));
            }
            Some(_) => {}
            None => root = Some(top.to_owned()),
        }
        let mut current = stage.try_clone()?;
        for directory in directories {
            match rustix::fs::mkdirat(&current, *directory, Mode::from_raw_mode(0o755)) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(error) => return Err(error.into()),
            }
            current = open_directory(&current, directory)?;
        }
        let file = rustix::fs::openat(
            &current,
            *name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o644),
        )?;
        let mut file = std::fs::File::from(file);
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    root.ok_or_else(|| std::io::Error::other("no modules to export"))
}

/// Move `base/root` aside to `base/previous` and `base/staging/root` into
/// its place.
fn swap(base: &OwnedFd, staging: &str, root: &str, previous: &str) -> std::io::Result<()> {
    let stage = open_directory(base, staging)?;
    match rustix::fs::renameat(base, root, base, previous) {
        Ok(()) | Err(Errno::NOENT) => {}
        Err(error) => return Err(error.into()),
    }
    match rustix::fs::renameat(&stage, root, base, root) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Put the old tree back. If another export took its place, the
            // old tree is removed with the staging leftovers instead.
            rustix::fs::renameat(base, previous, base, root).unwrap_or_default();
            Err(if matches!(error, Errno::EXIST | Errno::NOTEMPTY) {
                std::io::Error::other(format!(
                    "another export put its {root}/ in place while this one ran; run it again"
                ))
            } else {
                error.into()
            })
        }
    }
}

fn open_directory(parent: &OwnedFd, name: &str) -> std::io::Result<OwnedFd> {
    Ok(rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?)
}

/// Distinct for every export, even concurrent ones in one process.
fn unique() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!(
        "{}.{time}.{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}
