//! The name is what the boot selects: every entry holds its
//! basename even where it contributes no bytes.

use super::{boot_bytes, boot_override, system_files_in};
use std::path::PathBuf;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("ruxel-sysctl-select-{}-{name}", std::process::id()));
        drop(std::fs::remove_dir_all(&path));
        std::fs::create_dir_all(&path).expect("operation succeeds");
        Self(path)
    }

    fn dir(&self, name: &str) -> PathBuf {
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

fn dirs_of(etc: &std::path::Path, usr: &std::path::Path) -> (String, String) {
    (
        etc.to_str().expect("path is text").to_owned(),
        usr.to_str().expect("path is text").to_owned(),
    )
}

#[test]
fn a_link_to_a_directory_holds_the_name_at_boot() {
    // procps records the entry's basename before it tries to open
    // it: a higher-precedence `*.conf` link to an existing
    // directory holds the name although `is_file`, following the
    // link, answers false — the old scan dropped the name and
    // admitted the lower-precedence copy, whose assignments procps
    // never loads. The selection is the name alone; the unreadable
    // entry loads as nothing.
    let scratch = Scratch::new("dir-link");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let target = scratch.dir("target");
    std::os::unix::fs::symlink(target, etc.join("99-vendor.conf"))
        .expect("the link points at a directory");
    std::fs::write(usr.join("99-vendor.conf"), "net.ipv4.ip_forward=1\n")
        .expect("operation succeeds");
    let (etc_dir, usr_dir) = dirs_of(&etc, &usr);
    let dirs: [(&str, u8); 2] = [(&etc_dir, 1), (&usr_dir, 0)];
    let files = system_files_in(&dirs);
    assert!(
        files.iter().any(|file| file == &etc.join("99-vendor.conf")),
        "the link holds the name: {files:?}"
    );
    assert!(
        !files.iter().any(|file| file.starts_with(&usr)),
        "the shadowed copy must not load: {files:?}"
    );
    // The selected entry contributes no assignments, so the managed
    // value stands: only the lower copy the boot never loads
    // assigns another value.
    let host: Vec<(PathBuf, Vec<u8>)> = files
        .iter()
        .map(|file| {
            (
                file.clone(),
                boot_bytes(file)
                    .expect("the surface classifies")
                    .unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        boot_override(
            "/run/sysctl.d/10-ruxel.conf",
            "net.ipv4.ip_forward",
            "0",
            &host,
            &[]
        ),
        None,
        "the shadowed assignment must not hand the boot value away"
    );
}

#[test]
fn a_directory_holds_the_name_at_boot() {
    // A real subdirectory named `*.conf` is no openable file
    // either, and procps selects its basename all the same: the
    // lower-precedence copy of the name never loads and weighs
    // nothing on the boot value.
    let scratch = Scratch::new("dir-entry");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    std::fs::create_dir_all(etc.join("99-vendor.conf")).expect("operation succeeds");
    std::fs::write(usr.join("99-vendor.conf"), "net.ipv4.ip_forward=1\n")
        .expect("operation succeeds");
    let (etc_dir, usr_dir) = dirs_of(&etc, &usr);
    let dirs: [(&str, u8); 2] = [(&etc_dir, 1), (&usr_dir, 0)];
    let files = system_files_in(&dirs);
    assert!(
        files.iter().any(|file| file == &etc.join("99-vendor.conf")),
        "the directory holds the name: {files:?}"
    );
    assert!(
        !files.iter().any(|file| file.starts_with(&usr)),
        "the shadowed copy must not load: {files:?}"
    );
    let host: Vec<(PathBuf, Vec<u8>)> = files
        .iter()
        .map(|file| {
            (
                file.clone(),
                boot_bytes(file)
                    .expect("the surface classifies")
                    .unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        boot_override(
            "/run/sysctl.d/10-ruxel.conf",
            "net.ipv4.ip_forward",
            "0",
            &host,
            &[]
        ),
        None,
        "the shadowed assignment must not hand the boot value away"
    );
}
