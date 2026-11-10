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
        "import \"modulepath:/ruxel/engine/Engine.pkl\" as R\noutput { value = new R.Literal { value = \"inside\" }.value }",
    ] {
        fixture.write("main.pkl", source)?;
        let expected = crate::eval(&fixture.root().join("main.pkl"), &[])?;
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

mod cases;
mod denial;
