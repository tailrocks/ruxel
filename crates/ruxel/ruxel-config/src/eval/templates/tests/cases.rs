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
    let mut modules = Modules::new(Arc::new(Confined::new(vec![root.0.try_clone()?])));
    let dependency = root.0.path().join("dependency.pkl");
    assert_eq!(modules.read(&dependency)?, "value = \"first\"");
    fixture.write("dependency.pkl", "value = \"second\"")?;
    assert_eq!(
        modules.read(&root.0.path().join("sub/../dependency.pkl"))?,
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
fn file_resources_are_confined_to_the_template_root()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    std::fs::write(fixture.0.join("outside.txt"), "outside-value")?;
    fixture.write("inside.txt", "inside-value")?;
    let root = TemplateRoot::open(&fixture.root())?;
    fixture.write("main.pkl", "output { value = read(\"inside.txt\").text }")?;
    assert_eq!(root.evaluate(Path::new("main.pkl"))?, json!("inside-value"));
    for source in [
        "output { value = read(\"../outside.txt\").text }",
        &format!(
            "output {{ value = read(\"file:{}\").text }}",
            fixture.0.join("outside.txt").display()
        ),
    ] {
        fixture.write("main.pkl", source)?;
        let result = root.evaluate(Path::new("main.pkl"));
        assert!(
            result
                .as_ref()
                .is_err_and(|error| error.to_string().contains("outside the configuration root")),
            "{result:?}"
        );
    }
    Ok(())
}
