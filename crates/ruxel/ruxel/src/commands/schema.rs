use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// Write every served module for editors below `dir` (for example
/// `dir/ruxel/Ruxel.pkl`).
///
/// The whole tree is staged before any module is replaced, so a failure
/// while creating or writing leaves an older export as it was. Each staging
/// file is created exclusively, so a file or symlink planted under its name
/// is refused instead of followed, and no directory below `dir` may be a
/// symlink, so nothing is written outside it.
pub(crate) fn export(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut staged = Vec::new();
    let result = stage(dir, &mut staged).and_then(|()| publish(&staged));
    if result.is_err() {
        for (staging, _) in &staged {
            drop(std::fs::remove_file(staging));
        }
    }
    result
}

/// Write each module to a staging file beside its target.
fn stage(dir: &Path, staged: &mut Vec<(PathBuf, PathBuf)>) -> std::io::Result<()> {
    for (path, text) in ruxel_config::schema::exported_modules() {
        let relative = Path::new(path.trim_start_matches('/'));
        let parent = directories_beneath(dir, relative.parent().unwrap_or(Path::new("")))?;
        let name = relative
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let staging = parent.join(format!(".{name}.{}", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        staged.push((staging, parent.join(&name)));
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    Ok(())
}

/// Replace every target with its staging file.
fn publish(staged: &[(PathBuf, PathBuf)]) -> std::io::Result<()> {
    for (staging, target) in staged {
        std::fs::rename(staging, target)?;
    }
    Ok(())
}

/// Create `relative`'s directories below `dir`, refusing any that is a
/// symlink or not a directory.
fn directories_beneath(dir: &Path, relative: &Path) -> std::io::Result<PathBuf> {
    let mut current = dir.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(std::io::Error::other(format!(
                "unexpected module path component in {}",
                relative.display()
            )));
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(std::io::Error::other(format!(
                    "{} is a symlink; refusing to export through it",
                    current.display()
                )));
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(std::io::Error::other(format!(
                    "{} is not a directory",
                    current.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(current)
}
