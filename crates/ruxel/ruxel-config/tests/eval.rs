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
    assert!(
        ruxel_config::eval(
            Path::new("/nonexistent.pkl"),
            &ruxel_config::EvalOptions::default()
        )
        .is_err()
    );
}

#[test]
fn served_schema_is_admitted_and_its_fingerprint_removed() -> Result {
    let scratch = Scratch::new()?;
    let root = scratch.write("main.pkl", &config(ruxel_config::schema::ENGINE_URI))?;
    let raw = ruxel_config::eval(&root, &ruxel_config::EvalOptions::default())?;
    let served = raw["config"]["schemaFingerprint"]
        .as_str()
        .ok_or("no fingerprint")?;
    assert!(served.starts_with(&format!("{}@", ruxel_config::schema::fingerprint())));
    let admitted = ruxel_config::eval_config(&root, &ruxel_config::EvalOptions::default())?;
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
        include_str!("../../../../schema/ruxel/engine/Engine.pkl"),
    )?;
    // An export from another ruxel: a different fingerprint.
    let other = format!("blake3:{}", "0".repeat(64));
    scratch.write(
        "stale/ruxel/engine/Engine.pkl",
        &ruxel_config::schema::exported_source()
            .replace(ruxel_config::schema::fingerprint(), &other),
    )?;
    // This binary's own export, edited without touching the fingerprint.
    scratch.write(
        "edited/ruxel/engine/Engine.pkl",
        &ruxel_config::schema::exported_source().replace(
            "port: Int(this >= 1 && this <= 65535) = 22",
            "port: Int = 2222",
        ),
    )?;
    for (import, found) in [
        ("copied/Ruxel.pkl", "unserved"),
        ("stale/ruxel/engine/Engine.pkl", other.as_str()),
        (
            "edited/ruxel/engine/Engine.pkl",
            "a copy of this ruxel's schema",
        ),
    ] {
        let root = scratch.write("main.pkl", &config(import))?;
        let error = ruxel_config::eval_config(&root, &ruxel_config::EvalOptions::default())
            .err()
            .ok_or("copy admitted")?
            .to_string();
        assert!(error.contains(found), "{error}");
        assert!(
            error.contains(ruxel_config::schema::fingerprint()),
            "{error}"
        );
        assert!(
            error.contains("modulepath:/ruxel/engine/Engine.pkl"),
            "{error}"
        );
    }
    Ok(())
}

#[test]
fn module_path_directories_serve_libraries_but_never_the_schema() -> Result {
    let scratch = Scratch::new()?;
    scratch.write(
        "lib/roles/hello.pkl",
        "import \"modulepath:/ruxel/engine/Engine.pkl\" as R\nmessage = \"hello\"\nfingerprint = new R.Config { inventory {} }.schemaFingerprint\n",
    )?;
    // A directory entry with its own /ruxel/engine/Engine.pkl, such as an export.
    scratch.write("lib/ruxel/engine/Engine.pkl", "shadowed = true\n")?;
    let root = scratch.write(
        "main.pkl",
        "import \"modulepath:/roles/hello.pkl\" as hello\nmessage = hello.message\nfingerprint = hello.fingerprint\n",
    )?;
    let value = ruxel_config::eval(
        &root,
        &ruxel_config::EvalOptions {
            module_path: vec![scratch.0.join("lib")],
            root_dir: None,
        },
    )?;
    assert_eq!(value["message"], "hello");
    assert!(value["fingerprint"].as_str().is_some_and(|served| {
        served.starts_with(&format!("{}@", ruxel_config::schema::fingerprint()))
    }));
    assert!(ruxel_config::eval(&root, &ruxel_config::EvalOptions::default()).is_err());
    Ok(())
}

#[test]
fn module_path_libraries_read_their_resources() -> Result {
    let scratch = Scratch::new()?;
    scratch.write("lib/assets/greeting.txt", "hello")?;
    scratch.write(
        "lib/assets/lib.pkl",
        "greeting = read(\"greeting.txt\").text\n",
    )?;
    let root = scratch.write(
        "main.pkl",
        "import \"modulepath:/assets/lib.pkl\" as lib\ngreeting = lib.greeting\n",
    )?;
    let value = ruxel_config::eval(
        &root,
        &ruxel_config::EvalOptions {
            module_path: vec![scratch.0.join("lib")],
            root_dir: None,
        },
    )?;
    assert_eq!(value["greeting"], "hello");
    Ok(())
}
