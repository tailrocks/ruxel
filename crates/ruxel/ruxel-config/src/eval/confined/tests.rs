use crate::{EvalOptions, eval};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// `<tmp>/outside.txt` next to a configuration directory `<tmp>/config`.
pub(super) struct Fixture(PathBuf);

impl Fixture {
    pub(super) fn new() -> std::io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ruxel-confined-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("config"))?;
        std::fs::write(path.join("outside.txt"), "outside")?;
        std::fs::write(path.join("outside.pkl"), "value = \"outside\"")?;
        Ok(Self(path))
    }

    pub(super) fn config(&self) -> PathBuf {
        self.0.join("config")
    }

    pub(super) fn write(&self, name: &str, contents: &str) -> std::io::Result<PathBuf> {
        let path = self.config().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, contents)?;
        Ok(path)
    }

    pub(super) fn eval(&self, source: &str) -> std::result::Result<serde_json::Value, String> {
        let path = self
            .write("main.pkl", source)
            .map_err(|error| error.to_string())?;
        eval(&path, &EvalOptions::default()).map_err(|error| error.to_string())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

pub(super) fn refused(result: std::result::Result<serde_json::Value, String>) -> bool {
    result.is_err_and(|error| {
        error.contains("outside the configuration root") || error.contains("through a symlink")
    })
}

#[test]
fn imports_and_resources_inside_the_root_work() -> Result {
    let fixture = Fixture::new()?;
    fixture.write(
        "roles/role.pkl",
        "import \"../lib.pkl\" as lib\nvalue = lib.value + read(\"../keys/key.asc\").text",
    )?;
    fixture.write("lib.pkl", "value = \"lib+\"")?;
    fixture.write("keys/key.asc", "key")?;
    fixture.write("files/a.txt", "a")?;
    fixture.write("files/b.txt", "b")?;
    assert_eq!(
        fixture.eval("import \"roles/role.pkl\" as role\nvalue = role.value\nfiles = read*(\"files/*.txt\").keys.toList()")?,
        json!({"value": "lib+key", "files": ["files/a.txt", "files/b.txt"]})
    );
    Ok(())
}

#[test]
fn reads_and_imports_cannot_leave_the_root() -> Result {
    let fixture = Fixture::new()?;
    let outside = fixture.0.join("outside.txt");
    for source in [
        "value = read(\"../outside.txt\").text".to_owned(),
        "value = read(\"sub/../../outside.txt\").text".to_owned(),
        format!("value = read(\"file:{}\").text", outside.display()),
        format!("value = read?(\"file:{}\")?.text", outside.display()),
        "value = read(\"file:/etc/hosts\").text".to_owned(),
        "import \"../outside.pkl\" as o\nvalue = o.value".to_owned(),
        "value = import(\"../outside.pkl\").value".to_owned(),
    ] {
        assert!(refused(fixture.eval(&source)), "{source}");
    }
    Ok(())
}

#[test]
fn root_dir_widens_the_root_for_modules_in_subdirectories() -> Result {
    let fixture = Fixture::new()?;
    fixture.write("inventory.pkl", "hosts = 1")?;
    let root = fixture.write(
        "rehearsal/main.pkl",
        "import \"../inventory.pkl\" as i\nhosts = i.hosts",
    )?;
    assert!(eval(&root, &EvalOptions::default()).is_err());
    let options = EvalOptions {
        root_dir: Some(fixture.config()),
        ..EvalOptions::default()
    };
    assert_eq!(eval(&root, &options)?, json!({"hosts": 1}));
    let elsewhere = EvalOptions {
        root_dir: Some(fixture.config().join("rehearsal")),
        ..EvalOptions::default()
    };
    assert!(
        eval(&fixture.write("main.pkl", "hosts = 1")?, &elsewhere)
            .is_err_and(|error| error.to_string().contains("outside --root-dir"))
    );
    Ok(())
}

#[test]
fn missing_root_module_is_an_error() {
    assert!(eval(Path::new("/nonexistent/main.pkl"), &EvalOptions::default()).is_err());
}

#[cfg(unix)]
mod symlinks;
