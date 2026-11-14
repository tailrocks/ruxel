//! The broken link's name: procps selects it, cannot open it, and
//! never loads the lower-precedence copy of the name.

use super::super::boot;
use super::boot_surface::Scratch;

#[test]
fn a_broken_link_shadows_the_lower_precedence_name() {
    // procps selects one file per name from the highest-precedence
    // directory that holds it: `sysctl --dry-run --system` reports the
    // broken link it cannot open and never loads the same-name lower
    // copy, so the name is what shadows and the task must refuse the
    // file whose entry is skipped at boot while it reports success —
    // and name presence is what `symlink_metadata` answers, never
    // `Path::exists`, which follows the link and answers false.
    let scratch = Scratch::new("broken-shadow");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let broken = etc.join("50-ruxel.conf");
    std::os::unix::fs::symlink(scratch.0.join("missing"), &broken)
        .expect("the link points nowhere");
    let managed = scratch.file("usr", "50-ruxel.conf", "net.ipv4.ip_forward=1\n");
    let etc_dir = format!("{}/", etc.to_str().expect("path is text"));
    let usr_dir = format!("{}/", usr.to_str().expect("path is text"));
    let dirs: [(&str, u8); 2] = [(etc_dir.as_str(), 1), (usr_dir.as_str(), 0)];
    assert_eq!(
        boot::shadowing_in(managed.to_str().expect("path is text"), &dirs),
        Some(broken.to_str().expect("path is text").to_string()),
        "the broken link holds the name and shadows the managed file"
    );
}

#[test]
fn a_broken_link_holds_the_name_at_boot() {
    // The name selection the shadow check reasons with: procps loads
    // the broken higher-precedence link's name once, reports that it
    // cannot open it, and never the lower copy — so the link holds its
    // place like the `/dev/null` mask and the copy must not weigh on
    // the boot value.
    let scratch = Scratch::new("broken-name");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let broken = etc.join("99-vendor.conf");
    std::os::unix::fs::symlink(scratch.0.join("missing"), &broken)
        .expect("the link points nowhere");
    let vendor = scratch.file("usr", "99-vendor.conf", "net.ipv4.ip_forward=1\n");
    let dirs: [(&str, u8); 2] = [
        (etc.to_str().expect("path is text"), 1),
        (usr.to_str().expect("path is text"), 0),
    ];
    let files = boot::system_files_in(&dirs);
    assert!(
        files.iter().any(|file| file == &broken),
        "the broken link holds the name: {files:?}"
    );
    assert!(
        !files.iter().any(|file| file == &vendor),
        "the shadowed copy must not load: {files:?}"
    );
}
