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
    let files = vec![first.clone()];
    let before = boot::fingerprint_of(&files).expect("fingerprint");
    let second = scratch.file("etc", "99-other.conf", "net.ipv4.ip_forward=0\n");
    let after = boot::fingerprint_of(&[files[0].clone(), second]).expect("fingerprint");
    assert_ne!(before, after, "a new boot file moves the surface");
    let changed = scratch.file("etc", "90-ruxel.conf", "net.ipv4.ip_forward=0\n");
    assert_ne!(
        before,
        boot::fingerprint_of(&[changed]).expect("fingerprint"),
        "a changed boot file moves the surface"
    );
}

// APFS refuses a file name that is not text, so the surface is
// provable only where procps reads it: the Linux lane runs this, and
// the code keeps the raw name bytes on every platform.
#[cfg(target_os = "linux")]
#[test]
fn a_file_whose_name_is_not_text_is_in_the_boot_surface() {
    use std::os::unix::ffi::OsStringExt;
    // A `*.conf` basename need not be valid UTF-8 and procps reads it
    // all the same — `sysctl --dry-run --system` prints its assignment
    // — so such a file weighs on the surface like any other. Dropping
    // it would hide both the override it makes and the move it gives
    // the fingerprint, while a cached task keeps reporting success.
    let scratch = Scratch::new("nontext");
    let dir = scratch.dir("etc");
    let mut name = std::ffi::OsString::from_vec(b"\xffbad".to_vec());
    name.push(".conf");
    let path = dir.join(&name);
    std::fs::write(&path, "net.ipv4.ip_forward=0\n").expect("operation succeeds");
    let text = dir.to_str().expect("the scratch path is text").to_owned();
    let dirs = [(text.as_str(), 4u8)];
    let files = boot::system_files_in(&dirs);
    assert!(files.contains(&path), "{files:?}");
    // and the fingerprint over that surface is not the empty one
    let without = boot::fingerprint_of(&[]).expect("an empty surface fingerprints");
    let with = boot::fingerprint_of(&files).expect("the surface fingerprints");
    assert_ne!(without, with, "the file must weigh on the fingerprint");
}

#[test]
fn a_later_file_keeps_only_its_final_assignment() {
    // procps applies a file's lines in order and the last assignment
    // wins: a later file that assigns `=0` and then `=1` hands boot the
    // value `1`, so a task managing `1` must not be refused for the
    // intermediate value the file itself overrides again.
    let host = vec![(
        "/etc/sysctl.conf".to_string(),
        b"net.ipv4.ip_forward=0\nnet.ipv4.ip_forward=1\n".to_vec(),
    )];
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &host,
            &[],
        ),
        None
    );
    // A final differing assignment still hands the boot value away.
    let host = vec![(
        "/etc/sysctl.conf".to_string(),
        b"net.ipv4.ip_forward=1\nnet.ipv4.ip_forward=0\n".to_vec(),
    )];
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.ip_forward",
            "1",
            &host,
            &[],
        ),
        Some(("/etc/sysctl.conf".to_string(), "0".to_string()))
    );
}

#[test]
fn a_wildcard_stays_within_its_path_component() {
    // procps expands a wildcard one segment at a time: `net.*` covers
    // one component, so `net.*.rp_filter` assigns no
    // `net.ipv4.conf.all.rp_filter`, while `net.ipv4.conf.*.rp_filter`
    // expands to the interface keys and assigns them.
    let host = vec![(
        "/etc/sysctl.conf".to_string(),
        b"net.*.rp_filter=0\n".to_vec(),
    )];
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.conf.all.rp_filter",
            "1",
            &host,
            &[],
        ),
        None
    );
    let host = vec![(
        "/etc/sysctl.conf".to_string(),
        b"net.ipv4.conf.*.rp_filter=0\n".to_vec(),
    )];
    assert_eq!(
        boot::boot_override(
            "/etc/sysctl.d/90-ruxel.conf",
            "net.ipv4.conf.all.rp_filter",
            "1",
            &host,
            &[],
        ),
        Some(("/etc/sysctl.conf".to_string(), "0".to_string()))
    );
}
