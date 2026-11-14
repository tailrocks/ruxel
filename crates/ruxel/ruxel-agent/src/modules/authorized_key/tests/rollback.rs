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
