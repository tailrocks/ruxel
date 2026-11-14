use std::ffi::CString;
use std::path::PathBuf;

fn scratch(tag: &str) -> PathBuf {
    let base = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ruxel-file-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

#[test]
fn a_name_that_no_longer_holds_the_directory_is_not_taken_away() {
    let base = scratch("replaced");
    let dir = base.join("dir");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("file"), b"content").unwrap();
    let parent = std::os::fd::OwnedFd::from(std::fs::File::open(&base).unwrap());
    let child = std::os::fd::OwnedFd::from(std::fs::File::open(&dir).unwrap());
    // The target is renamed away, and a replacement takes its name.
    std::fs::rename(&dir, base.join("moved")).unwrap();
    std::fs::create_dir(&dir).unwrap();
    // The name does not hold the directory that was opened: the
    // removal must refuse, and the tree of the moved directory stays.
    assert!(!super::super::attachment::holds_name(&parent, c"dir", &child).unwrap());
    let error =
        super::super::attachment::take_name(&parent, &CString::new("dir").unwrap(), &child, &dir)
            .unwrap_err();
    assert!(error.contains("replaced"), "{error}");
    // The replacement is back under the name it had.
    assert!(dir.exists());
    assert_eq!(std::fs::read(base.join("moved/file")).unwrap(), b"content");
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn a_name_that_holds_the_directory_is_taken_away() {
    let base = scratch("attached");
    let dir = base.join("dir");
    std::fs::create_dir(&dir).unwrap();
    let parent = std::os::fd::OwnedFd::from(std::fs::File::open(&base).unwrap());
    let child = std::os::fd::OwnedFd::from(std::fs::File::open(&dir).unwrap());
    assert!(super::super::attachment::holds_name(&parent, c"dir", &child).unwrap());
    let staging =
        super::super::attachment::take_name(&parent, &CString::new("dir").unwrap(), &child, &dir)
            .unwrap();
    // The name is gone, and the directory is under the name of its own.
    assert!(!dir.exists());
    assert!(base.join(staging.to_str().unwrap()).exists());
    std::fs::remove_dir_all(base).unwrap();
}
