//! The boot surface: the system directories procps reads, the one copy
//! of a name it loads, and the fingerprint of the surface the ledger
//! weighs on its cached results.

use std::path::PathBuf;

use super::super::boot;

/// A scratch directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("ruxel-sysctl-boot-{}-{name}", std::process::id()));
        drop(std::fs::remove_dir_all(&path));
        std::fs::create_dir_all(&path).expect("operation succeeds");
        Self(path)
    }

    fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(&path).expect("operation succeeds");
        path
    }

    fn file(&self, dir: &str, name: &str, content: &str) -> PathBuf {
        let path = self.0.join(dir).join(name);
        std::fs::create_dir_all(self.0.join(dir)).expect("operation succeeds");
        std::fs::write(&path, content).expect("operation succeeds");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

#[test]
fn every_procps_system_directory_is_read() {
    // procps reads five directories and the legacy file
    // (`sysctl.conf(5)`): a file in `/usr/local/lib/sysctl.d` or
    // `/lib/sysctl.d` assigns a key at boot like any other.
    let dirs: Vec<&str> = boot::DIRS.iter().map(|(dir, _)| *dir).collect();
    assert_eq!(
        dirs,
        vec![
            "/etc/sysctl.d/",
            "/run/sysctl.d/",
            "/usr/local/lib/sysctl.d/",
            "/usr/lib/sysctl.d/",
            "/lib/sysctl.d/",
        ]
    );
}

#[test]
fn a_shadowed_copy_of_a_name_is_not_part_of_the_boot_surface() {
    // procps loads one file per name, from the highest-precedence
    // directory that holds it: the shadowed copy never reaches the
    // kernel and must not weigh on the boot value.
    let scratch = Scratch::new("shadowed");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    scratch.file("etc", "99-vendor.conf", "net.ipv4.ip_forward=1\n");
    scratch.file("usr", "99-vendor.conf", "net.ipv4.ip_forward=0\n");
    let dirs: [(&str, u8); 2] = [
        (etc.to_str().expect("path is text"), 1),
        (usr.to_str().expect("path is text"), 0),
    ];
    let files = boot::system_files_in(&dirs);
    assert_eq!(files.len(), 2, "one copy of the name, and the legacy file");
    assert!(
        files
            .iter()
            .any(|file| file.starts_with(etc.to_str().expect("path is text"))),
        "the highest-precedence copy is the loaded one: {files:?}"
    );
    assert!(
        !files
            .iter()
            .any(|file| file.starts_with(usr.to_str().expect("path is text"))),
        "the shadowed copy is not loaded: {files:?}"
    );
}

#[test]
fn the_boot_fingerprint_moves_with_the_boot_files() {
    // The fingerprint is what the ledger weighs on its cached result: a
    // file added or changed after the task ran moves the boot value the
    // task never judged.
    let scratch = Scratch::new("fingerprint");
    let first = scratch.file("etc", "90-ruxel.conf", "net.ipv4.ip_forward=1\n");
    let files = vec![first.to_string_lossy().into_owned()];
    let before = boot::fingerprint_of(&files).expect("fingerprint");
    let second = scratch.file("etc", "99-other.conf", "net.ipv4.ip_forward=0\n");
    let after = boot::fingerprint_of(&[files[0].clone(), second.to_string_lossy().into_owned()])
        .expect("fingerprint");
    assert_ne!(before, after, "a new boot file moves the surface");
    let changed = scratch.file("etc", "90-ruxel.conf", "net.ipv4.ip_forward=0\n");
    assert_ne!(
        before,
        boot::fingerprint_of(&[changed.to_string_lossy().into_owned()]).expect("fingerprint"),
        "a changed boot file moves the surface"
    );
}
