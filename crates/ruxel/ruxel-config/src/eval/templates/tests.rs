use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "ruxel-template-modules-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path)?;
        std::fs::create_dir(path.join("modules"))?;
        Ok(Self(path))
    }

    fn root(&self) -> PathBuf {
        self.0.join("modules")
    }

    fn write(&self, path: &str, source: &str) -> std::io::Result<()> {
        std::fs::write(self.root().join(path), source)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

#[test]
fn ordinary_transitive_expression_and_inherited_imports_work()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    std::fs::create_dir(fixture.root().join("sub"))?;
    fixture.write("common.pkl", "value = \"inside\"")?;
    fixture.write(
        "sub/dependency.pkl",
        "import \"../common.pkl\" as common\nvalue = common.value",
    )?;
    fixture.write(
        "base.pkl",
        "open module base\nvalue = \"inside\"\noutput { value = value }",
    )?;
    let root = TemplateRoot::open(&fixture.root())?;
    for source in [
        "import \"sub/dependency.pkl\" as dep\noutput { value = dep.value }",
        "output { value = import(\"common.pkl\").value }",
        "amends \"base.pkl\"",
        "extends \"base.pkl\"",
        "import \"pkl:base\" as base\noutput { value = \"inside\" }",
    ] {
        fixture.write("main.pkl", source)?;
        let expected = crate::eval(&fixture.root().join("main.pkl"))?;
        assert_eq!(root.evaluate(Path::new("main.pkl"))?, expected, "{source}");
    }
    Ok(())
}

#[test]
fn parser_rejects_all_external_and_glob_forms_before_evaluation()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    std::fs::write(
        fixture.0.join("outside.pkl"),
        "output { value = \"outside\" }",
    )?;
    let root = TemplateRoot::open(&fixture.root())?;
    for source in [
        "import \"../outside.pkl\" as outside\noutput { value = outside }",
        "amends \"../outside.pkl\"",
        "extends \"../outside.pkl\"",
        "output { value = import(\"../outside.pkl\") }",
        "import* \"*.pkl\" as all\noutput { value = \"unused\" }",
        "output { value = import*(\"*.pkl\") }",
        r#"output { value = "\(import*("*.pkl"))" }"#,
        "import \".../outside.pkl\" as outside\noutput { value = \"unused\" }",
        "import \"https://example.invalid/outside.pkl\" as outside\noutput { value = \"unused\" }",
        "import \"file:outside.pkl\" as outside\noutput { value = \"unused\" }",
        "import \"package://example.invalid/pkg@1#/outside.pkl\" as outside\noutput { value = \"unused\" }",
    ] {
        fixture.write("main.pkl", source)?;
        assert!(
            root.evaluate(Path::new("main.pkl")).is_err(),
            "accepted {source}"
        );
    }
    fixture.write(
        "dependency.pkl",
        "import \"../outside.pkl\" as outside\nvalue = \"unused\"",
    )?;
    fixture.write(
        "main.pkl",
        "import \"dependency.pkl\" as dep\noutput { value = dep.value }",
    )?;
    assert!(root.evaluate(Path::new("main.pkl")).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn imported_file_and_directory_symlinks_are_rejected()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new()?;
    std::fs::write(fixture.0.join("outside.pkl"), "value = \"outside\"")?;
    symlink(
        fixture.0.join("outside.pkl"),
        fixture.root().join("link.pkl"),
    )?;
    symlink(&fixture.0, fixture.root().join("linkdir"))?;
    let root = TemplateRoot::open(&fixture.root())?;
    for dependency in ["link.pkl", "linkdir/outside.pkl"] {
        fixture.write(
            "main.pkl",
            &format!("import \"{dependency}\" as dep\noutput {{ value = dep.value }}"),
        )?;
        assert!(
            root.evaluate(Path::new("main.pkl")).is_err(),
            "accepted {dependency}"
        );
    }
    Ok(())
}

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

#[test]
fn provider_denial_survives_evaluator_fallbacks() {
    struct Denied;
    impl ModuleSource for Denied {
        fn identity(&mut self, _: &Path) -> Result<PathBuf> {
            Err(Error::Eval("synthetic denial".into()))
        }
        fn exists(&mut self, _: &Path) -> Result<bool> {
            Err(Error::Eval("synthetic denial".into()))
        }
        fn read(&mut self, _: &Path) -> Result<String> {
            Err(Error::Eval("synthetic denial".into()))
        }
    }
    let mut evaluator = super::super::builder().build();
    evaluator.set_module_source(Denied);
    let error = evaluator.eval_source_json_blocking(
        "output { value = \"otherwise valid\" }",
        Path::new("main.pkl"),
    );
    assert!(error.is_err());
}

#[test]
fn missing_inheritance_is_an_error() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let root = TemplateRoot::open(&fixture.root())?;
    for verb in ["amends", "extends"] {
        fixture.write(
            "main.pkl",
            &format!("{verb} \"missing.pkl\"\noutput {{ value = \"otherwise valid\" }}"),
        )?;
        assert!(
            root.evaluate(Path::new("main.pkl")).is_err(),
            "accepted {verb}"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinked_intermediate_inheritance_is_an_error()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    std::fs::write(
        fixture.0.join("base.pkl"),
        "open module base\nvalue = \"outside\"",
    )?;
    std::os::unix::fs::symlink(&fixture.0, fixture.root().join("linked"))?;
    let root = TemplateRoot::open(&fixture.root())?;
    for verb in ["amends", "extends"] {
        fixture.write(
            "main.pkl",
            &format!("{verb} \"linked/base.pkl\"\noutput {{ value = \"otherwise valid\" }}"),
        )?;
        assert!(
            root.evaluate(Path::new("main.pkl")).is_err(),
            "accepted {verb}"
        );
    }
    Ok(())
}

#[test]
fn rendering_cannot_catch_and_hide_module_provider_denial() {
    struct DenyReads;
    impl ModuleSource for DenyReads {
        fn identity(&mut self, path: &Path) -> Result<PathBuf> {
            Ok(path.into())
        }
        fn exists(&mut self, _: &Path) -> Result<bool> {
            Ok(true)
        }
        fn read(&mut self, _: &Path) -> Result<String> {
            Err(Error::Eval("rendering-only denial".into()))
        }
    }
    let source = r#"import "pkl:test" as test
output {
  value = "visible"
  renderer = new JsonRenderer {
    converters { [String] = (s) -> test.catchOrNull(() -> import("dependency.pkl").value) ?? s }
  }
}"#;
    let mut evaluator = super::super::builder().build();
    evaluator.set_module_source(DenyReads);
    assert!(evaluator.eval_source(source, Path::new("main.pkl")).is_ok());
    let error = evaluator.eval_source_json_blocking(source, Path::new("main.pkl"));
    assert!(error.is_err_and(|error| error.to_string().contains("rendering-only denial")));
}

#[cfg(unix)]
#[test]
fn racing_dependency_symlinks_never_supply_outside_source()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    use std::sync::{Arc, atomic::AtomicBool};
    let fixture = Fixture::new()?;
    fixture.write("dependency.pkl", "value = \"inside\"")?;
    fixture.write(
        "main.pkl",
        "import \"dependency.pkl\" as dep\noutput { value = dep.value }",
    )?;
    let outside = fixture.0.join("outside.pkl");
    std::fs::write(&outside, "value = \"outside\"")?;
    let root = TemplateRoot::open(&fixture.root())?;
    assert_eq!(root.evaluate(Path::new("main.pkl"))?, json!("inside"));
    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = Arc::clone(&stop);
    let dependency = fixture.root().join("dependency.pkl");
    let held = fixture.root().join("held.pkl");
    let writer = std::thread::spawn(move || {
        while !writer_stop.load(Ordering::Relaxed) {
            if std::fs::rename(&dependency, &held).is_ok() {
                drop(std::os::unix::fs::symlink(&outside, &dependency));
                drop(std::fs::remove_file(&dependency));
                drop(std::fs::rename(&held, &dependency));
            }
        }
    });
    let results = (0..100)
        .map(|_| root.evaluate(Path::new("main.pkl")))
        .collect::<Vec<_>>();
    stop.store(true, Ordering::Relaxed);
    assert!(writer.join().is_ok());
    for value in results.into_iter().flatten() {
        assert_eq!(value, json!("inside"));
    }
    Ok(())
}
