//! The `.conf` dotfile is a boot file: `Path::extension` reports no
//! extension for a basename consisting entirely of `.conf`, yet procps
//! loads that dotfile.

use super::scratch::{Scratch, dirs_of};
use super::system_files_in;
#[test]
fn a_dot_conf_dotfile_is_a_boot_file() {
    // Thread 335: `Path::extension` reports no extension for a
    // basename consisting entirely of `.conf`, yet procps loads that
    // dotfile — `sysctl --dry-run --system` prints `* Applying
    // /run/sysctl.d/.conf ...`. The scan matches the raw basename's
    // `.conf` suffix, or an unmanaged `.conf` that sorts after an
    // earlier managed file overrides its key invisibly.
    let scratch = Scratch::new("dot-conf");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    std::fs::write(etc.join(".conf"), "net.ipv4.ip_forward=1\n").expect("operation succeeds");
    std::fs::write(etc.join("README"), "not a sysctl file\n").expect("operation succeeds");
    std::fs::write(etc.join("stray.conf.bak"), "net.ipv4.ip_forward=1\n")
        .expect("operation succeeds");
    let (etc_dir, usr_dir) = dirs_of(&etc, &usr);
    let dirs: [(&str, u8); 2] = [(&etc_dir, 1), (&usr_dir, 0)];
    let files = system_files_in(&dirs).expect("the scratch directories read");
    assert!(
        files.iter().any(|file| file == &etc.join(".conf")),
        "the .conf dotfile loads at boot: {files:?}"
    );
    assert!(
        !files.iter().any(|file| file == &etc.join("README")),
        "a bare name stays out: {files:?}"
    );
    assert!(
        !files.iter().any(|file| file == &etc.join("stray.conf.bak")),
        "a backup suffix stays out: {files:?}"
    );
}
