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

#[test]
fn private_staging_is_distinct_and_descriptor_anchored() {
    let root = Root::new();
    let dest = root.0.join("result");
    let first = Download::new(&dest).unwrap();
    let second = Download::new(&dest).unwrap();
    write_bytes(&first.file).unwrap();
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 2);
    for entry in fs::read_dir(&root.0).unwrap() {
        assert_eq!(
            entry.unwrap().metadata().unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let renamed = root.0.with_extension("moved");
    fs::rename(&root.0, &renamed).unwrap();
    fs::create_dir(&root.0).unwrap();
    first.publish(&dest).unwrap();
    assert!(!dest.exists());
    assert_eq!(fs::read(renamed.join("result")).unwrap(), b"download\0\xff");
    drop(first);
    drop(second);
    assert_eq!(fs::read_dir(&renamed).unwrap().count(), 1);
    fs::remove_dir_all(renamed).unwrap();
}

#[test]
fn renamed_stage_cannot_redirect_publication_through_symlink() {
    let root = Root::new();
    let dest = root.0.join("result");
    let stage = Download::new(&dest).unwrap();
    let original = fs::read_dir(&root.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let held = root.0.join("held");
    let victim = root.0.join("victim");
    fs::create_dir(&victim).unwrap();
    fs::write(victim.join("payload"), "unchanged").unwrap();
    fs::rename(&original, &held).unwrap();
    symlink(&victim, &original).unwrap();
    write_bytes(&stage.file).unwrap();
    stage.publish(&dest).unwrap();
    drop(stage);
    assert_eq!(fs::read(&dest).unwrap(), b"download\0\xff");
    assert_eq!(fs::read(victim.join("payload")).unwrap(), b"unchanged");
    assert_eq!(fs::read_dir(held).unwrap().count(), 0);
}

#[test]
fn clients_write_to_stdout_descriptor_and_wget_remains_fallback() {
    let root = Root::new();
    let script = root.0.join("client");
    fs::write(&script, "#!/bin/sh\ncase \"$1\" in\n-fsSL) test \"$2\" = -- || exit 9;;\n-q) test \"$2\" = -O && test \"$3\" = - && test \"$4\" = -- || exit 10;;\n*) exit 11;;\nesac\nprintf 'client bytes'\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    for curl in [&script, &root.0.join("missing")] {
        let stage = Download::new(&root.0.join("result")).unwrap();
        assert!(
            super::fetch_with_programs("https://example.invalid/file", &stage.file, curl, &script)
                .unwrap()
        );
        stage.publish(Path::new("result")).unwrap();
        assert_eq!(fs::read(root.0.join("result")).unwrap(), b"client bytes");
        fs::remove_file(root.0.join("result")).unwrap();
    }
}
