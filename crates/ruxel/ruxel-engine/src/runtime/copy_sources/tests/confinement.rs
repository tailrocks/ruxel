use super::*;

#[test]
fn source_root_remains_pinned_after_directory_replacement() -> Result<()> {
    let directory = Directory::new()?;
    let original = directory.path().join("source");
    let moved = directory.path().join("moved");
    std::fs::create_dir(&original)?;
    std::fs::write(original.join("asset"), b"original")?;
    std::fs::write(
        original.join("template.pkl"),
        "output { value = \"original\" }",
    )?;
    let root = SourceRoot::open(&original)?;
    std::fs::rename(&original, &moved)?;
    std::fs::create_dir(&original)?;
    std::fs::write(original.join("asset"), b"replacement")?;
    std::fs::write(
        original.join("template.pkl"),
        "output { value = \"replacement\" }",
    )?;
    let template_root = root.template_root(&original)?;
    assert_eq!(
        template_root.evaluate(Path::new("template.pkl"))?,
        "original"
    );
    let mut copy = task("asset");
    super::super::prepare(&mut copy, &root)?;
    assert_eq!(
        copy.iterations[0].source_content.as_deref(),
        Some(&b"original"[..])
    );
    Ok(())
}

#[test]
fn copy_sources_are_confined_to_root_without_following_symlinks() -> Result<()> {
    use std::os::unix::fs::symlink;
    let directory = Directory::new()?;
    let outside = Directory::new()?;
    std::fs::write(outside.path().join("outside"), b"outside")?;
    std::fs::create_dir(directory.path().join("nested"))?;
    std::fs::write(directory.path().join("nested/inside"), b"inside")?;
    symlink(outside.path(), directory.path().join("linkdir"))?;
    symlink(
        outside.path().join("outside"),
        directory.path().join("linkfile"),
    )?;
    symlink(
        directory.path().join("nested/inside"),
        directory.path().join("internal-link"),
    )?;

    let mut normal = task("nested/inside");
    prepare(&mut normal, directory.path())?;
    assert_eq!(
        normal.iterations[0].source_content.as_deref(),
        Some(&b"inside"[..])
    );
    for source in [
        "../outside",
        "nested/../nested/inside",
        "linkdir/outside",
        "linkfile",
        "internal-link",
        "",
    ] {
        let mut attempt = task(source);
        assert!(
            prepare(&mut attempt, directory.path()).is_err(),
            "source {source:?} should fail"
        );
        assert!(attempt.iterations[0].source_content.is_none());
    }
    let absolute_path = outside.path().join("outside");
    let mut absolute = task(
        absolute_path
            .to_str()
            .context("absolute path is not UTF-8")?,
    );
    assert!(prepare(&mut absolute, directory.path()).is_err());
    Ok(())
}
