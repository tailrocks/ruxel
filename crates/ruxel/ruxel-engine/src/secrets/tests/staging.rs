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
    let error = StagedValue::stage(PLAINTEXT, |file, _path, bytes| {
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

/// A write that fails partway has already put the secret's bytes on the
/// disk, and the removal that follows is part of that failure, not a
/// cleanup detail to drop on the floor: when the removal itself fails,
/// the error says so — naming the removal, never the path, which is
/// where the plaintext is — instead of the generic staging error alone.
#[test]
fn a_partial_write_failure_reports_a_removal_that_failed() {
    const PLAINTEXT: &str = "synthetic-partial-stage-cleanup-3f81-a4d6";
    let mut sabotaged: Option<std::path::PathBuf> = None;
    let error = StagedValue::stage(PLAINTEXT, |file, path, bytes| {
        use std::io::Write;
        file.write_all(&bytes[..bytes.len() / 2])?;
        // The removal the cleanup attempts cannot succeed: the staging
        // path holds a directory now, and `remove_file` on a directory
        // fails.
        std::fs::remove_file(path)?;
        std::fs::create_dir(path)?;
        sabotaged = Some(path.to_path_buf());
        Err(std::io::Error::other("synthetic disk full"))
    })
    .expect_err("the staging write failed");
    assert!(
        error.contains("could not be removed"),
        "names the removal: {error}"
    );
    assert!(
        !error.contains(".ruxel-fnox-stage"),
        "the error never names the staging path: {error}"
    );
    // The plaintext is gone: the file that held the partial bytes is
    // not on the disk any more.
    let leaked = staging_files_containing(PLAINTEXT);
    assert!(
        leaked.is_empty(),
        "the failed write left its bytes behind: {leaked:?}"
    );
    // The directory the test put in the staging path goes now.
    let path = sabotaged.expect("the staging path");
    std::fs::remove_dir(&path).expect("remove the directory in the staging path");
}

/// A store that never starts leaves the staging file behind: the spawn
/// failed and the file holds the plaintext. The removal is part of that
/// failure, and a removal that fails is reported — naming the removal,
/// never the path — so the plaintext is never left on the disk with only
/// a generic process error.
#[test]
fn a_spawn_failure_reports_a_removal_that_failed() {
    let mut sabotaged: Option<std::path::PathBuf> = None;
    let error = ProcessBackend::fnox_set_with("KEY", "synthetic-spawn-stage-plaintext", |staged| {
        let path = staged.path();
        // The store never starts, and the removal that follows
        // cannot succeed: the staging path holds a directory now.
        std::fs::remove_file(path)?;
        std::fs::create_dir(path)?;
        sabotaged = Some(path.to_path_buf());
        Err(std::io::Error::other("synthetic spawn failure"))
    })
    .expect_err("the store could not start");
    assert!(
        error.contains("could not be removed"),
        "names the removal: {error}"
    );
    assert!(
        !error.contains(".ruxel-fnox-stage"),
        "the error never names the staging path: {error}"
    );
    // The plaintext is gone: the file that held it is not on the disk
    // any more.
    let leaked = staging_files_containing("synthetic-spawn-stage-plaintext");
    assert!(
        leaked.is_empty(),
        "the failed spawn left the plaintext behind: {leaked:?}"
    );
    // The directory the test put in the staging path goes now.
    let path = sabotaged.expect("the staging path");
    std::fs::remove_dir(&path).expect("remove the directory in the staging path");
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

/// A staging file orphaned by abrupt termination is recovered by the
/// next store, not left to accumulate: the naming pattern attributes the
/// file to a process, and a process that is gone will never drop it, so
/// the next store removes it before it stages its own value. A staging
/// file a live process may still be using is left alone — a concurrent
/// run's store is not a stale file.
#[test]
fn a_staging_file_orphaned_by_a_terminated_run_is_recovered_on_the_next_store() {
    // A process that has been reaped: its `Drop` can never run again.
    let mut gone = std::process::Command::new("true").spawn().expect("spawn");
    let dead = gone.id();
    gone.wait().expect("wait");
    // A concurrent run that is still alive owns its staging files.
    let mut live_run = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("spawn");
    let live = live_run.id();

    let orphan = std::env::temp_dir().join(format!(".ruxel-fnox-stage-{dead}-0"));
    let owned = std::env::temp_dir().join(format!(".ruxel-fnox-stage-{live}-0"));
    std::fs::write(&orphan, "synthetic-orphan-stage-plaintext").expect("write the orphan");
    std::fs::write(&owned, "synthetic-live-stage-plaintext").expect("write the live staging");

    let staged = StagedValue::new("synthetic-recovery-store").expect("the next store stages");

    assert!(
        !orphan.exists(),
        "the staging file of the terminated run was left to accumulate"
    );
    assert!(owned.exists(), "the staging file of a live run was taken");
    let _result = std::fs::remove_file(&owned);
    let _result = live_run.kill();
    let _result = live_run.wait();
    drop(staged);
}
