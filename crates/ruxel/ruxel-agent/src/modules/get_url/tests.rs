use super::{ExecContext, run_with_fetch, staging::Download};
use serde_json::json;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Root(pub(super) PathBuf);

impl Root {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ruxel-download-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn context(check_mode: bool) -> ExecContext {
    ExecContext {
        check_mode,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    }
}

fn write_bytes(mut file: &File) -> Result<bool, String> {
    file.write_all(b"download\0\xff")
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[test]
fn planted_legacy_symlink_is_never_opened() {
    let root = Root::new();
    let victim = root.0.join("victim");
    let dest = root.0.join("result");
    fs::write(&victim, "unchanged").unwrap();
    symlink(&victim, root.0.join(".result.ruxel-dl")).unwrap();
    let params = json!({"url":"https://example.invalid/file", "dest":dest});
    assert_eq!(
        run_with_fetch(&params, &context(false), |_, file| write_bytes(file)).unwrap()["changed"],
        true
    );
    assert_eq!(fs::read(&dest).unwrap(), b"download\0\xff");
    assert_eq!(fs::read(&victim).unwrap(), b"unchanged");
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 3);
    run_with_fetch(&params, &context(false), |_, _| panic!("converged fetch")).unwrap();
}

#[test]
fn failure_and_check_leave_no_download_objects() {
    let root = Root::new();
    let params = json!({"url":"https://example.invalid/file", "dest":root.0.join("result")});
    run_with_fetch(&params, &context(true), |_, _| panic!("check fetch")).unwrap();
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    let result = run_with_fetch(&params, &context(false), |_, file| {
        write_bytes(file)?;
        Err("synthetic failed download".into())
    });
    assert_eq!(result.unwrap_err(), "synthetic failed download");
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    assert!(run_with_fetch(&params, &context(false), |_, _| Ok(false)).is_err());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
}

#[path = "tests/staging_cases.rs"]
mod staging_cases;
