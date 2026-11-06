use super::{Ledger, Path};
use std::io::{Read, Write};

impl Ledger {
    pub(crate) fn load(state_dir: &Path) -> Self {
        let ledger_dir = state_dir.join("ledger");
        drop(std::fs::create_dir_all(&ledger_dir));
        set_mode(&ledger_dir, 0o700);
        let key_path = ledger_dir.join("key");
        let had_host_key = key_path.exists();
        let host_key = load_or_create_host_key(&key_path);
        let path = ledger_dir.join("ledger.json");
        // A ledger without its host key is legacy or unrecoverable. Remove it:
        // legacy result payloads may contain pre-no_log plaintext diffs.
        if !had_host_key {
            drop(std::fs::remove_file(&path));
        }
        set_mode(&path, 0o600);
        let records = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Ledger {
            path,
            host_key,
            records,
            dirty: false,
        }
    }

    pub(crate) fn flush(&self) {
        if !self.dirty {
            return;
        }
        if let Some(parent) = self.path.parent() {
            drop(std::fs::create_dir_all(parent));
            set_mode(parent, 0o700);
        }
        if let Ok(bytes) = serde_json::to_vec(&self.records) {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, &bytes).is_ok() {
                set_mode(&tmp, 0o600);
                drop(std::fs::rename(&tmp, &self.path));
                set_mode(&self.path, 0o600);
            }
        }
    }
}

fn load_or_create_host_key(path: &Path) -> Option<[u8; 32]> {
    if let Ok(bytes) = std::fs::read(path) {
        return bytes.try_into().ok();
    }

    let mut key = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .ok()?
        .read_exact(&mut key)
        .ok()?;

    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    };
    #[cfg(not(unix))]
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path);

    match file {
        Ok(mut file) => {
            file.write_all(&key).ok()?;
            file.sync_all().ok()?;
            set_mode(path, 0o600);
            Some(key)
        }
        Err(_) => std::fs::read(path).ok()?.try_into().ok(),
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    drop(std::fs::set_permissions(
        path,
        std::fs::Permissions::from_mode(mode),
    ));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}
