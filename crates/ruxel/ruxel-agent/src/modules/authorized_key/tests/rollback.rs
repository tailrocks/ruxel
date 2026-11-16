use super::*;

#[test]
fn a_rollback_does_not_take_a_newer_edit_away() {
    let root = scratch("rollback");
    let meta = std::fs::metadata(&root).unwrap();
    let home = File::open(&root).unwrap();
    let ssh = open_ssh(&home, meta.uid(), meta.gid(), true)
        .unwrap()
        .unwrap();
    // The file that is read.
    std::fs::write(root.join(".ssh/authorized_keys"), b"old\n").unwrap();
    let (_, identity) = read_authorized_keys_with_identity(&ssh).unwrap();
    // Another writer changes the file before the publication: the
    // publication must refuse, because the file it was written from is
    // not the one that is there.
    std::fs::write(root.join(".ssh/authorized_keys"), b"other\n").unwrap();
    // A writer that does not take the lock acts while the publication
    // is in flight.
    let racer = || {
        std::fs::write(root.join(".ssh/authorized_keys"), b"new\n").unwrap();
    };
    let error = publish::publish_racing(
        &ssh,
        b"published\n",
        meta.uid(),
        meta.gid(),
        identity,
        &racer,
    )
    .unwrap_err();
    assert!(error.contains("changed during operation"), "{error}");
    // The edit of the writer that does not take the lock is still
    // there: the rollback did not take it away.
    assert_eq!(
        std::fs::read_to_string(root.join(".ssh/authorized_keys")).unwrap(),
        "new\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_rollback_does_not_lose_the_edit_of_a_writer_that_acts_in_it() {
    let root = scratch("rollback-race");
    let meta = std::fs::metadata(&root).unwrap();
    let home = File::open(&root).unwrap();
    let ssh = open_ssh(&home, meta.uid(), meta.gid(), true)
        .unwrap()
        .unwrap();
    // The file that is read.
    std::fs::write(root.join(".ssh/authorized_keys"), b"old\n").unwrap();
    let (_, identity) = read_authorized_keys_with_identity(&ssh).unwrap();
    // Another writer changes the file before the publication: the
    // publication must refuse, because the file it was written from is
    // not the one that is there.
    std::fs::write(root.join(".ssh/authorized_keys"), b"other\n").unwrap();
    // A writer that does not take the lock acts while the rollback is
    // in flight: the rollback must not take its edit away, and must
    // not strand it either.
    let racer = || {
        std::fs::write(root.join(".ssh/authorized_keys"), b"raced\n").unwrap();
    };
    let error = publish::publish_racing_at(
        &ssh,
        b"published\n",
        meta.uid(),
        meta.gid(),
        identity,
        &|| {},
        &racer,
    )
    .unwrap_err();
    assert!(error.contains("changed during operation"), "{error}");
    // The edit of the writer that does not take the lock is still
    // there: the rollback did not take it away.
    assert_eq!(
        std::fs::read_to_string(root.join(".ssh/authorized_keys")).unwrap(),
        "raced\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_rollback_restores_the_file_that_was_there() {
    let root = scratch("rollback-restore");
    let meta = std::fs::metadata(&root).unwrap();
    let home = File::open(&root).unwrap();
    let ssh = open_ssh(&home, meta.uid(), meta.gid(), true)
        .unwrap()
        .unwrap();
    std::fs::write(root.join(".ssh/authorized_keys"), b"old\n").unwrap();
    let (_, identity) = read_authorized_keys_with_identity(&ssh).unwrap();
    // The file is changed before the publication: the publication
    // refuses, and the file that is there goes back where it was.
    std::fs::write(root.join(".ssh/authorized_keys"), b"other\n").unwrap();
    let error = publish::publish_checked(&ssh, b"published\n", meta.uid(), meta.gid(), identity)
        .unwrap_err();
    assert!(error.contains("changed during operation"), "{error}");
    assert_eq!(
        std::fs::read_to_string(root.join(".ssh/authorized_keys")).unwrap(),
        "other\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_displaced_entry_that_cannot_be_inspected_is_rolled_back_too() {
    let root = scratch("rollback-unreadable");
    let meta = std::fs::metadata(&root).unwrap();
    let home = File::open(&root).unwrap();
    let ssh = open_ssh(&home, meta.uid(), meta.gid(), true)
        .unwrap()
        .unwrap();
    std::fs::write(root.join(".ssh/authorized_keys"), b"old\n").unwrap();
    let (_, identity) = read_authorized_keys_with_identity(&ssh).unwrap();
    // A writer that does not take the lock replaces the file with a
    // link while the publication is in flight: the displaced entry is
    // a link, and the inspection that reads it fails. The staged key
    // must not stay at the path when apply reports failure, and the
    // link of the writer must not be stranded in the staging
    // directory.
    let racer = || {
        let staging = std::fs::read_dir(root.join(".ssh"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .find(|name| name.to_string_lossy().starts_with(".ruxel-key-"))
            .expect("the staging directory must be there");
        let key = root.join(".ssh").join(staging).join("key");
        std::fs::remove_file(&key).unwrap();
        std::os::unix::fs::symlink("elsewhere", &key).unwrap();
    };
    let error = publish::publish_racing(
        &ssh,
        b"published\n",
        meta.uid(),
        meta.gid(),
        identity,
        &racer,
    )
    .unwrap_err();
    assert!(error.contains("changed during operation"), "{error}");
    // The link the writer put in is back at the path, and the staged
    // key is not active at it.
    let path = root.join(".ssh/authorized_keys");
    let entry = std::fs::symlink_metadata(&path).unwrap();
    assert!(entry.file_type().is_symlink(), "{entry:?}");
    assert_eq!(
        std::fs::read_link(&path).unwrap(),
        std::path::Path::new("elsewhere")
    );
    // Nothing is left behind in the directory of the user.
    let stranded: Vec<_> = std::fs::read_dir(root.join(".ssh"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(".ruxel-key-"))
        .collect();
    assert!(stranded.is_empty(), "{stranded:?}");
    std::fs::remove_dir_all(root).unwrap();
}
