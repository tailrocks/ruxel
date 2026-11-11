//! The staging file a store write travels in: it holds the plaintext, so
//! it is removed on every path out of the store — a write that fails
//! partway included.

use super::super::staging::StagedValue;
use super::generation::Fixture;
use super::*;

/// The staging files of this process whose content carries `marker`: the
/// temp directory is shared — across runs and across tests — so a leak is
/// named by this process's name and the bytes it holds, never by which
/// file appeared.
fn staging_files_containing(marker: &str) -> Vec<std::path::PathBuf> {
    let prefix = format!(".ruxel-fnox-stage-{}-", std::process::id());
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let leaked = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(&prefix))
            && std::fs::read(&path).is_ok_and(|bytes| {
                bytes
                    .windows(marker.len())
                    .any(|window| window == marker.as_bytes())
            });
        if leaked {
            found.push(path);
        }
    }
    found
}

/// A write that fails partway has already put the secret's bytes on the
/// disk: the cleanup guard is installed the moment the file exists, before
/// the first byte is written, so the failure path of the write removes the
/// file with the bytes that did reach it. A guard constructed only after a
/// successful write would never run on that path — the error return would
/// leave the partial plaintext behind.
#[test]
fn a_partial_write_failure_removes_the_partially_written_file() {
    const PLAINTEXT: &str = "synthetic-partial-stage-4f2c-9d21";
    let half = &PLAINTEXT.as_bytes()[..PLAINTEXT.len() / 2];
    let error = StagedValue::stage(PLAINTEXT, |file, bytes| {
        use std::io::Write;
        file.write_all(&bytes[..bytes.len() / 2])?;
        Err(std::io::Error::other("synthetic disk full"))
    })
    .expect_err("the staging write failed");
    assert_eq!(error, "store staging failed", "names the staging: {error}");
    let leaked =
        staging_files_containing(std::str::from_utf8(half).expect("half of an ascii marker"));
    assert!(
        leaked.is_empty(),
        "a failed write left its partially written bytes behind: {leaked:?}"
    );
}

/// A staging file that cannot be removed fails the store: the file holds
/// the complete secret, so a removal failure is not a cleanup detail the
/// store may report success over. The fake store takes the value, then
/// removes the staging file itself and puts a directory in its place —
/// the filesystem failure the controller's cleanup then meets. The
/// error names the removal and never the path, which is where the
/// plaintext is.
#[test]
fn a_staging_removal_failure_fails_the_store_without_naming_the_path() {
    let fixture = Fixture::new();
    let fnox = fixture.script(
        "sabotaging-fnox",
        r#"store="${0%/*}/store"
if [ "$1" = "set" ] && [ "$#" -eq 4 ] && [ "$3" = "--from-file" ]; then
  mkdir -p "$store"
  cat "$4" > "$store/$2"
  rm -f "$4"
  mkdir "$4"
  exit 0
fi
exit 8"#,
    );
    let backend = ProcessBackend::with_programs(fnox, "/nonexistent/ruxel-op");
    let error = backend
        .fnox_set("KEY", "synthetic-staged-plaintext")
        .expect_err("the staging file could not be removed");
    assert!(
        error.contains("could not be removed"),
        "names the removal: {error}"
    );
    assert!(
        !error.contains(".ruxel-fnox-stage"),
        "the error never names the staging path: {error}"
    );
    // The store itself succeeded: the failure the run reports is the
    // plaintext it was left with, not the store call.
    assert_eq!(
        std::fs::read(fixture.directory.join("store").join("KEY")).expect("stored"),
        b"synthetic-staged-plaintext",
        "the value reached the store before the cleanup failed"
    );
    // The directory the fake store left in the staging path goes now; the
    // best-effort drop could not remove it, which is what the error said.
    let staging_prefix = format!(".ruxel-fnox-stage-{}-", std::process::id());
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&staging_prefix))
                && entry.path().is_dir()
            {
                drop(std::fs::remove_dir(entry.path()));
            }
        }
    }
}

/// A staged value is removed on every path: the drop of a staging whose
/// store never ran, the explicit cleanup that reports a failure when there
/// is one, and the drop of a value whose cleanup already ran remove the
/// file — and a file that is already gone counts as removed.
#[test]
fn a_staged_value_is_removed_on_every_path() {
    let staged = StagedValue::new("synthetic-staged-plaintext").expect("stage the value");
    let path = staged.path().to_path_buf();
    assert!(path.is_file(), "the staging file exists while staged");
    drop(staged);
    assert!(!path.exists(), "the drop removed the staging file");
    let mut staged = StagedValue::new("synthetic-staged-plaintext").expect("stage the value");
    staged.cleanup().expect("the cleanup removes the file");
    assert!(!staged.path().exists(), "the file is gone after cleanup");
    // A second cleanup of a value whose file is already gone succeeds:
    // removal of what is not there is the guarantee, not a failure.
    staged.cleanup().expect("a gone file counts as removed");
    drop(staged);
}
