use std::io::Write;
use std::path::Path;

/// Write every served module for editors below `dir` (for example
/// `dir/ruxel/Ruxel.pkl`), replacing older exports atomically, file by file.
///
/// Each staging file is created exclusively, so a file or symlink planted
/// under its name is refused instead of followed.
pub(crate) fn export(dir: &Path) -> std::io::Result<()> {
    for (path, text) in ruxel_config::schema::exported_modules() {
        let target = dir.join(path.trim_start_matches('/'));
        let parent = target.parent().unwrap_or(dir);
        std::fs::create_dir_all(parent)?;
        let name = target
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let staging = parent.join(format!(".{name}.{}", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        let written = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .and_then(|()| std::fs::rename(&staging, &target));
        if written.is_err() {
            drop(std::fs::remove_file(&staging));
            return written;
        }
    }
    Ok(())
}
