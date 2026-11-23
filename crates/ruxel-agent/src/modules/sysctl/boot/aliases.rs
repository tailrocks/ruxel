//! The self-alias the shadowing probe exempts: a dangling link that
//! names the managed file itself.

use super::scratch::Scratch;

#[test]
fn a_dangling_relative_alias_is_no_shadow() {
    // Thread 297: a higher-precedence relative link to a managed file
    // that does not exist yet compares literally and mismatches —
    // normalizing the lexical `..` components admits the genuine
    // self-alias the publish creates, instead of refusing a valid
    // bootstrap layout as a shadow.
    let scratch = Scratch::new("alias");
    let etc = scratch.dir("etc");
    let usr = scratch.dir("usr");
    let candidate = etc.join("x.conf");
    std::os::unix::fs::symlink("../usr/x.conf", &candidate).expect("operation succeeds");
    let file = usr.join("x.conf");
    assert!(
        !file.exists(),
        "both sides dangle: the managed file is not published yet"
    );
    assert!(
        super::shadow::is_self_alias(
            candidate.to_str().expect("path is text"),
            file.to_str().expect("path is text")
        ),
        "the dangling relative alias is the managed file itself"
    );
    // A dangling link anywhere else is still a shadow.
    let elsewhere = etc.join("y.conf");
    std::os::unix::fs::symlink("../usr/z.conf", &elsewhere).expect("operation succeeds");
    assert!(
        !super::shadow::is_self_alias(
            elsewhere.to_str().expect("path is text"),
            file.to_str().expect("path is text")
        ),
        "a dangling link elsewhere still shadows"
    );
}

#[test]
fn a_symlink_beneath_a_dangling_alias_is_no_self_alias() {
    // Thread 297: a `..` past a link resolves from the link's
    // target, not from the spelling — collapsing it lexically across
    // the link names a file the entry never reaches. Here the
    // candidate's target crosses the `link` component, which points at
    // a live directory elsewhere: lexically the alias names the
    // managed file, but the loader resolves it past the link's target
    // instead, so the dangling pair must refuse rather than exempt
    // the shadow.
    let scratch = Scratch::new("alias-symlink");
    let elsewhere = Scratch::new("alias-elsewhere");
    let sub = scratch.dir("sub");
    let candidate = sub.join("x.conf");
    std::os::unix::fs::symlink("../link/../managed.conf", &candidate).expect("operation succeeds");
    std::os::unix::fs::symlink(elsewhere.0.clone(), sub.join("link")).expect("operation succeeds");
    let file = scratch.0.join("managed.conf");
    assert!(
        !candidate.exists() && !file.exists(),
        "both sides fail to resolve: the link target and the managed file are absent"
    );
    assert!(
        !super::shadow::is_self_alias(
            candidate.to_str().expect("path is text"),
            file.to_str().expect("path is text")
        ),
        "a dangling alias crossing a symlink is no self-alias"
    );
    // The same shape with no symlink beneath it stays a genuine
    // self-alias: the refusal above is the link, not the layout.
    let plain = sub.join("y.conf");
    std::os::unix::fs::symlink("../managed.conf", &plain).expect("operation succeeds");
    assert!(
        super::shadow::is_self_alias(
            plain.to_str().expect("path is text"),
            file.to_str().expect("path is text")
        ),
        "the link-free twin is still the managed file itself"
    );
}

#[test]
fn a_looping_alias_never_matches_lexically() {
    // Thread 297: only a missing file counts as dangling — a link
    // loop fails with a filesystem-loop error, not `NotFound`, so
    // even a lexically equal spelling refuses instead of exempting
    // the shadow.
    let scratch = Scratch::new("alias-loop");
    let sub = scratch.dir("sub");
    let candidate = sub.join("x.conf");
    std::os::unix::fs::symlink("x.conf", &candidate).expect("operation succeeds");
    let file = sub.join(".").join("..").join("sub").join("x.conf");
    assert!(
        !candidate.exists() && !file.exists(),
        "both sides fail to resolve: the loop never reaches a file"
    );
    assert!(
        !super::shadow::is_self_alias(
            candidate.to_str().expect("path is text"),
            file.to_str().expect("path is text")
        ),
        "a looping alias is no self-alias whatever the spelling"
    );
}
