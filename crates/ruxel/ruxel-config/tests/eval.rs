use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ruxel-eval-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn write(&self, relative: &str, source: &str) -> Result<PathBuf> {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().ok_or("no parent")?)?;
        std::fs::write(&path, source)?;
        Ok(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

fn config(import: &str) -> String {
    format!(
        "import \"{import}\" as R\nconfig = new R.Config {{\n  inventory {{ hosts {{ [\"node\"] = new R.Host {{ address = \"localhost\" }} }} }}\n}}\n"
    )
}

#[test]
fn missing_module_fails() {
    assert!(ruxel_config::eval(Path::new("/nonexistent.pkl"), &[]).is_err());
}

#[test]
fn served_schema_is_admitted_and_its_fingerprint_removed() -> Result {
    let scratch = Scratch::new()?;
    let root = scratch.write("main.pkl", &config(ruxel_config::schema::MODULE_URI))?;
    let raw = ruxel_config::eval(&root, &[])?;
    assert_eq!(
        raw["config"]["schemaFingerprint"],
        ruxel_config::schema::fingerprint()
    );
    let admitted = ruxel_config::eval_config(&root, &[])?;
    assert!(admitted["config"].get("schemaFingerprint").is_none());
    assert_eq!(
        admitted["config"]["inventory"]["hosts"]["node"]["address"],
        "localhost"
    );
    Ok(())
}

#[test]
fn copies_of_the_schema_are_refused() -> Result {
    let scratch = Scratch::new()?;
    // The committed source, copied: the fingerprint is the placeholder.
    scratch.write(
        "copied/Ruxel.pkl",
        include_str!("../../../../schema/Ruxel.pkl"),
    )?;
    // An export from another ruxel: a different fingerprint.
    scratch.write(
        "stale/ruxel/Ruxel.pkl",
        &ruxel_config::schema::source().replace(ruxel_config::schema::fingerprint(), "blake3:0123"),
    )?;
    for (import, found) in [
        ("copied/Ruxel.pkl", "unserved"),
        ("stale/ruxel/Ruxel.pkl", "blake3:0123"),
    ] {
        let root = scratch.write("main.pkl", &config(import))?;
        let error = ruxel_config::eval_config(&root, &[])
            .err()
            .ok_or("copy admitted")?
            .to_string();
        assert!(error.contains(found), "{error}");
        assert!(
            error.contains(ruxel_config::schema::fingerprint()),
            "{error}"
        );
        assert!(error.contains("modulepath:/ruxel/Ruxel.pkl"), "{error}");
    }
    Ok(())
}

#[test]
fn module_path_directories_serve_libraries_but_never_the_schema() -> Result {
    let scratch = Scratch::new()?;
    scratch.write(
        "lib/roles/hello.pkl",
        "import \"modulepath:/ruxel/Ruxel.pkl\" as R\nmessage = \"hello\"\nfingerprint = new R.Config { inventory {} }.schemaFingerprint\n",
    )?;
    // A directory entry with its own /ruxel/Ruxel.pkl, such as an export.
    scratch.write("lib/ruxel/Ruxel.pkl", "shadowed = true\n")?;
    let root = scratch.write(
        "main.pkl",
        "import \"modulepath:/roles/hello.pkl\" as hello\nmessage = hello.message\nfingerprint = hello.fingerprint\n",
    )?;
    let value = ruxel_config::eval(&root, &[scratch.0.join("lib")])?;
    assert_eq!(value["message"], "hello");
    assert_eq!(value["fingerprint"], ruxel_config::schema::fingerprint());
    assert!(ruxel_config::eval(&root, &[]).is_err());
    Ok(())
}
