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
fn a_symlink_alias_of_the_managed_file_is_no_shadow() {
    // The higher-precedence entry is a link to the managed file
    // itself: the boot reads the managed bytes through it
    // (`boot_bytes` follows links), so the name holds no shadow over
    // the file it already is.
    let scratch = Scratch::new("alias-exempt");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let managed = scratch.file("usr", "50-ruxel.conf", "net.ipv4.ip_forward=1\n");
    std::os::unix::fs::symlink(&managed, etc.join("50-ruxel.conf"))
        .expect("the link targets the managed file");
    let etc_dir = format!("{}/", etc.to_str().expect("path is text"));
    let usr_dir = format!("{}/", usr.to_str().expect("path is text"));
    let dirs: [(&str, u8); 2] = [(etc_dir.as_str(), 1), (usr_dir.as_str(), 0)];
    assert_eq!(
        boot::shadowing_in(managed.to_str().expect("path is text"), &dirs),
        None,
        "a link to the managed file is no shadow over it"
    );
}

#[test]
fn an_unrelated_symlink_still_shadows() {
    // A same-named link that resolves anywhere but the managed file —
    // here another file, and a `/dev/null` mask the same way — still
    // holds the name over the lower-precedence copy: only the link to
    // the managed file itself is exempt.
    let scratch = Scratch::new("unrelated-link");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let other = scratch.file("usr", "other.conf", "net.ipv4.ip_forward=0\n");
    let link = etc.join("50-ruxel.conf");
    std::os::unix::fs::symlink(&other, &link).expect("the link targets another file");
    let managed = scratch.file("usr", "50-ruxel.conf", "net.ipv4.ip_forward=1\n");
    let etc_dir = format!("{}/", etc.to_str().expect("path is text"));
    let usr_dir = format!("{}/", usr.to_str().expect("path is text"));
    let dirs: [(&str, u8); 2] = [(etc_dir.as_str(), 1), (usr_dir.as_str(), 0)];
    assert_eq!(
        boot::shadowing_in(managed.to_str().expect("path is text"), &dirs),
        Some(link.to_str().expect("path is text").to_string()),
        "a link to another file still shadows the managed file"
    );
    std::fs::remove_file(&link).expect("operation succeeds");
    std::os::unix::fs::symlink("/dev/null", &link).expect("the mask links to /dev/null");
    assert_eq!(
        boot::shadowing_in(managed.to_str().expect("path is text"), &dirs),
        Some(link.to_str().expect("path is text").to_string()),
        "a mask still shadows the managed file"
    );
}

#[test]
fn the_preflight_passes_the_alias_and_refuses_other_links() {
    // Integration through the shadow preflight: the alias link passes
    // without a declared shadow, while a broken link and an unrelated
    // one refuse and name the shadow.
    let scratch = Scratch::new("preflight-links");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let managed = scratch.file("usr", "50-ruxel.conf", "net.ipv4.ip_forward=1\n");
    let file = managed.to_str().expect("path is text").to_owned();
    let link = etc.join("50-ruxel.conf");
    let etc_dir = format!("{}/", etc.to_str().expect("path is text"));
    let usr_dir = format!("{}/", usr.to_str().expect("path is text"));
    let dirs: [(&str, u8); 2] = [(etc_dir.as_str(), 1), (usr_dir.as_str(), 0)];
    let empty: Vec<(String, String, String)> = Vec::new();

    std::os::unix::fs::symlink(&managed, &link).expect("the link targets the managed file");
    assert!(
        super::super::preflight_shadow_in(&file, "net.ipv4.ip_forward", &empty, &dirs).is_ok(),
        "the alias link passes the preflight"
    );

    std::fs::remove_file(&link).expect("operation succeeds");
    std::os::unix::fs::symlink(scratch.0.join("missing"), &link).expect("the link points nowhere");
    let error = super::super::preflight_shadow_in(&file, "net.ipv4.ip_forward", &empty, &dirs)
        .expect_err("a broken link must refuse");
    assert!(error.contains("shadows it at boot"), "{error}");

    std::fs::remove_file(&link).expect("operation succeeds");
    let other = scratch.file("usr", "other.conf", "net.ipv4.ip_forward=0\n");
    std::os::unix::fs::symlink(&other, &link).expect("the link targets another file");
    let error = super::super::preflight_shadow_in(&file, "net.ipv4.ip_forward", &empty, &dirs)
        .expect_err("an unrelated link must refuse");
    assert!(error.contains("shadows it at boot"), "{error}");
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
    let files = boot::system_files_in(&dirs).expect("the scratch directories read");
    assert!(
        files.iter().any(|file| file == &broken),
        "the broken link holds the name: {files:?}"
    );
    assert!(
        !files.iter().any(|file| file == &vendor),
        "the shadowed copy must not load: {files:?}"
    );
}
