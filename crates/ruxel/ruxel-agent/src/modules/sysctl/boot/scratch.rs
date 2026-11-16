//! Scratch directories and dir pairs for boot-surface tests.

use std::path::PathBuf;

pub(super) struct Scratch(pub(super) PathBuf);

impl Scratch {
    pub(super) fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("ruxel-sysctl-select-{}-{name}", std::process::id()));
        drop(std::fs::remove_dir_all(&path));
        std::fs::create_dir_all(&path).expect("operation succeeds");
        Self(path)
    }

    pub(super) fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(&path).expect("operation succeeds");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

pub(super) fn dirs_of(etc: &std::path::Path, usr: &std::path::Path) -> (String, String) {
    (
        etc.to_str().expect("path is text").to_owned(),
        usr.to_str().expect("path is text").to_owned(),
    )
}
