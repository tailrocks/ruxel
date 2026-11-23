use super::*;

#[test]
fn an_edit_in_place_is_refused_even_when_the_times_are_the_same() {
    let root = scratch("in-place");
    let meta = std::fs::metadata(&root).unwrap();
    let home = File::open(&root).unwrap();
    let ssh = open_ssh(&home, meta.uid(), meta.gid(), true)
        .unwrap()
        .unwrap();
    let path = root.join(".ssh/authorized_keys");
    std::fs::write(&path, b"old\n").unwrap();
    let (_, identity) = read_authorized_keys_with_identity(&ssh).unwrap();
    // The writer edits the file in place: the same length, and the
    // times of the file are put back, so that the file looks the way it
    // was read. The identity of the file holds the digest of its bytes,
    // so the edit is told from the file that was read.
    let before = std::fs::metadata(&path).unwrap();
    let modified = before.modified().unwrap();
    std::fs::write(&path, b"new\n").unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    drop(file);
    // The publication must refuse: the file it was written from is not
    // the one that is there.
    let error = publish::publish_checked(&ssh, b"published\n", meta.uid(), meta.gid(), identity)
        .unwrap_err();
    assert!(error.contains("changed during operation"), "{error}");
    // The edit of the writer is still there: the publication did not
    // take it away.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_identity_of_the_file_holds_the_digest_of_its_bytes() {
    let root = scratch("identity");
    let meta = std::fs::metadata(&root).unwrap();
    let home = File::open(&root).unwrap();
    let ssh = open_ssh(&home, meta.uid(), meta.gid(), true)
        .unwrap()
        .unwrap();
    let path = root.join(".ssh/authorized_keys");
    std::fs::write(&path, b"old\n").unwrap();
    let (_, identity) = read_authorized_keys_with_identity(&ssh).unwrap();
    // The file that is read is the file that is there: the identity
    // holds it.
    let again = read_authorized_keys_with_identity(&ssh).unwrap();
    assert_eq!(identity, again.1);
    // The bytes of the file are part of the identity: a file of the
    // same size and the same times is another file.
    let before = std::fs::metadata(&path).unwrap();
    let modified = before.modified().unwrap();
    std::fs::write(&path, b"new\n").unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    drop(file);
    let edited = read_authorized_keys_with_identity(&ssh).unwrap();
    assert_ne!(identity, edited.1);
    std::fs::remove_dir_all(root).unwrap();
}
