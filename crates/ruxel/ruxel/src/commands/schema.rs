use std::io::Write;
use std::path::Path;

/// Write the served schema to `dir/ruxel/Ruxel.pkl`, replacing an older
/// export atomically.
pub(crate) fn export(dir: &Path) -> std::io::Result<()> {
    let relative = ruxel_config::schema::MODULE_PATH.trim_start_matches('/');
    let target = dir.join(relative);
    let parent = target.parent().unwrap_or(dir);
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".Ruxel.pkl.{}", std::process::id()));
    let written = std::fs::File::create(&staging).and_then(|mut file| {
        file.write_all(ruxel_config::schema::source().as_bytes())?;
        file.sync_all()
    });
    match written.and_then(|()| std::fs::rename(&staging, &target)) {
        Ok(()) => Ok(()),
        Err(error) => {
            drop(std::fs::remove_file(&staging));
            Err(error)
        }
    }
}
