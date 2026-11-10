use std::io::Write;
use std::path::Path;

/// Write the schema for editors to `dir/ruxel/Ruxel.pkl`, replacing an older
/// export atomically.
///
/// The staging file is created exclusively, so a file or symlink planted
/// under its name is refused instead of followed.
pub(crate) fn export(dir: &Path) -> std::io::Result<()> {
    let relative = ruxel_config::schema::MODULE_PATH.trim_start_matches('/');
    let target = dir.join(relative);
    let parent = target.parent().unwrap_or(dir);
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".Ruxel.pkl.{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)?;
    let written = file
        .write_all(ruxel_config::schema::exported_source().as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| std::fs::rename(&staging, &target));
    if written.is_err() {
        drop(std::fs::remove_file(&staging));
    }
    written
}
