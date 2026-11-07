use super::{download_tests::Root, staging::Download};
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::process::Command;

#[test]
fn inherited_acl_is_removed_before_fetch_and_parent_acl_is_preserved() {
    let root = Root::new();
    let parent = root.0.join("parent");
    fs::DirBuilder::new().mode(0o700).create(&parent).unwrap();
    assert!(Command::new("/bin/chmod").args([
        "+a", "everyone allow list,add_file,search,delete_child,add_subdirectory,file_inherit,directory_inherit"
    ]).arg(&parent).status().unwrap().success());
    let params =
        serde_json::json!({"url":"https://example.invalid/file", "dest":parent.join("result")});
    let ctx = super::ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    };
    super::run_with_fetch(&params, &ctx, |_, mut file| {
        let staged = fs::read_dir(&parent)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let output = Command::new("/bin/ls")
            .arg("-lde")
            .arg(&staged)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap().lines().count(), 1);
        std::io::Write::write_all(&mut file, b"downloaded").unwrap();
        Ok(true)
    })
    .unwrap();
    assert_eq!(fs::read(parent.join("result")).unwrap(), b"downloaded");
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    let output = Command::new("/bin/ls")
        .arg("-lde")
        .arg(&parent)
        .output()
        .unwrap();
    assert!(String::from_utf8(output.stdout).unwrap().lines().count() > 1);
}

#[test]
fn acl_removal_uses_held_directory_after_name_retarget() {
    let root = Root::new();
    let stage = Download::new(&root.0.join("result")).unwrap();
    let original = fs::read_dir(&root.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let held = root.0.join("held");
    fs::rename(&original, &held).unwrap();
    fs::create_dir(&original).unwrap();
    assert!(
        Command::new("/bin/chmod")
            .args(["+a", "everyone allow add_file,search,delete_child"])
            .arg(&original)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("/bin/chmod")
            .args(["+a", "everyone allow add_file,search,delete_child"])
            .arg(&held)
            .status()
            .unwrap()
            .success()
    );
    super::access::clear_inherited_acl(&fs::File::open(&held).unwrap()).unwrap();
    let output = Command::new("/bin/ls")
        .arg("-lde")
        .arg(&held)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap().lines().count(), 1);
    let output = Command::new("/bin/ls")
        .arg("-lde")
        .arg(&original)
        .output()
        .unwrap();
    assert!(String::from_utf8(output.stdout).unwrap().lines().count() > 1);
    drop(stage);
}
