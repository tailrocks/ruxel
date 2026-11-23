//! The operator the agent runs as, read off a probe file the process
//! creates itself.

use std::os::unix::fs::MetadataExt;

/// The uid the agent operates as, read off a probe file the process
/// creates itself: the kernel owns new files to their creator, and the
/// workspace forbids the unsafe call that reads the euid directly. The
/// probe's name is unique to the process and created atomically
/// (`create_new`), so a planted file never stands in for it. Where no
/// probe can be created the answer matches nothing: only root-owned
/// directories are trusted then.
pub(crate) fn operating_uid() -> u32 {
    static OPERATOR: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *OPERATOR.get_or_init(|| probe_operating_uid().unwrap_or(u32::MAX))
}

fn probe_operating_uid() -> Result<u32, String> {
    let dir = std::env::temp_dir();
    for attempt in 0..100 {
        let probe = dir.join(format!("ruxel-operator-{}-{attempt}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(file) => {
                let uid = file.metadata().map(|meta| meta.uid());
                drop(file);
                let _ = std::fs::remove_file(&probe);
                return uid.map_err(|error| error.to_string());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("the operator probe could not be created".to_string())
}
