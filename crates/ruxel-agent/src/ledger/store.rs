use super::{Ledger, Path};
use std::collections::{HashMap, HashSet};
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
            workflow: String::new(),
            records,
            encountered: HashSet::new(),
            aborted: false,
            complete: false,
            dirty: false,
            #[cfg(test)]
            scans: std::cell::Cell::new(0),
        }
    }

    /// The plan has ended: every record of its own workflow it never
    /// encountered is history — a task a previous plan ran and this
    /// one does not — and expires before what the plan still owns is
    /// persisted. A complete plan that encountered nothing still
    /// expires everything of its own workflow: a workflow that dropped
    /// its last agent task runs no task of its own, and keeping its
    /// old records would leave them trusted — a removed sysctl
    /// declaration would keep authorizing its dropped task's
    /// still-present later file. Records of other workflows live in
    /// their own maps and are never this plan's verdict: the ledger is
    /// shared per host, so another workflow's completion must not
    /// delete them.
    ///
    /// Only a complete plan may expire what it never encountered. A
    /// plan that stopped early — a task failed and the plan halted on
    /// the failure — leaves the tasks it never reached in the workflow,
    /// and their records are the workflow's own: the next run must see
    /// the earlier stage's declaration of a later task, or it treats
    /// the later task's file as an unmanaged override and refuses the
    /// earlier stage before the later task can run. A session that
    /// ended without its end — the controller's stream closed mid-plan
    /// — is an executed prefix, not a complete plan, and expires
    /// nothing either.
    pub(crate) fn flush(&mut self) {
        if self.complete && !self.aborted {
            let before: usize = self.records.values().map(HashMap::len).sum();
            if let Some(owned) = self.records.get_mut(&self.workflow) {
                owned.retain(|key, _| self.encountered.contains(key));
            }
            self.records.retain(|_, owned| !owned.is_empty());
            let after: usize = self.records.values().map(HashMap::len).sum();
            self.dirty |= after != before;
        }
        // The plan is over: the next plan's encounters are its own.
        self.encountered.clear();
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
