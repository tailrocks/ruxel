use super::*;

#[test]
fn replacing_root_does_not_redirect_dependency_identity_existence_or_contents()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    fixture.write("dependency.pkl", "value = \"pinned\"")?;
    fixture.write(
        "main.pkl",
        "import \"dependency.pkl\" as dep\noutput { value = dep.value }",
    )?;
    let root = TemplateRoot::open(&fixture.root())?;
    std::fs::rename(fixture.root(), fixture.0.join("original"))?;
    std::fs::create_dir(fixture.root())?;
    fixture.write("dependency.pkl", "value = \"replacement\"")?;
    fixture.write("main.pkl", "output { value = \"replacement\" }")?;
    assert_eq!(root.evaluate(Path::new("main.pkl"))?, json!("pinned"));
    std::fs::remove_dir_all(fixture.root())?;
    assert_eq!(root.evaluate(Path::new("main.pkl"))?, json!("pinned"));
    Ok(())
}

#[test]
fn snapshots_are_consistent_within_evaluation_and_fresh_between_evaluations()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    fixture.write("dependency.pkl", "value = \"first\"")?;
    let root = TemplateRoot::open(&fixture.root())?;
    let mut modules = Modules {
        root: root.directory.try_clone()?,
        path: root.path.clone(),
        sources: BTreeMap::new(),
    };
    let dependency = root.path.join("dependency.pkl");
    assert_eq!(modules.read(&dependency)?, "value = \"first\"");
    fixture.write("dependency.pkl", "value = \"second\"")?;
    assert_eq!(
        modules.read(&root.path.join("sub/../dependency.pkl"))?,
        "value = \"first\""
    );
    std::fs::remove_file(&dependency)?;
    assert!(modules.exists(&dependency)?);
    assert_eq!(modules.read(&dependency)?, "value = \"first\"");
    fixture.write(
        "main.pkl",
        "import \"dependency.pkl\" as dep\noutput { value = dep.value }",
    )?;
    fixture.write("dependency.pkl", "value = \"fresh\"")?;
    assert_eq!(root.evaluate(Path::new("main.pkl"))?, json!("fresh"));
    Ok(())
}

#[test]
fn file_resource_reads_keep_native_behavior() -> std::result::Result<(), Box<dyn std::error::Error>>
{
    let fixture = Fixture::new()?;
    std::fs::write(fixture.0.join("resource.txt"), "resource-value")?;
    fixture.write(
        "main.pkl",
        "output { value = read(\"../resource.txt\").text }",
    )?;
    let path = fixture.root().join("main.pkl");
    let expected = crate::eval(&path)?;
    assert_eq!(expected, json!("resource-value"));
    assert_eq!(
        TemplateRoot::open(&fixture.root())?.evaluate(Path::new("main.pkl"))?,
        expected
    );
    Ok(())
}
