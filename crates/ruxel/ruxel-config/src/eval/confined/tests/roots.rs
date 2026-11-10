use super::{Fixture, Result};
use crate::{EvalOptions, eval};
use serde_json::json;
use std::path::Path;

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
fn nested_roots_read_through_their_own_descriptor() -> Result {
    use super::super::{Confined, Root};
    let fixture = Fixture::new()?;
    let lib = fixture.write("lib/value.txt", "pinned")?;
    let files = Confined::new(vec![
        Root::open(&fixture.config())?,
        Root::open(&fixture.config().join("lib"))?,
    ]);
    let lib = std::fs::canonicalize(lib)?;
    let detour = lib
        .parent()
        .and_then(Path::parent)
        .map(|config| config.join("other/../lib/value.txt"))
        .ok_or("fixture path")?;
    std::fs::rename(fixture.config().join("lib"), fixture.config().join("moved"))?;
    fixture.write("lib/value.txt", "replacement")?;
    assert_eq!(files.read(&lib)?, Some(b"pinned".to_vec()));
    assert_eq!(files.read(&detour)?, Some(b"pinned".to_vec()));
    Ok(())
}

#[test]
fn equal_root_paths_prefer_the_first_root() -> Result {
    use super::super::{Confined, Root};
    let fixture = Fixture::new()?;
    let file = fixture.write("lib/value.txt", "first")?;
    let first = Root::open(&fixture.config().join("lib"))?;
    std::fs::rename(fixture.config().join("lib"), fixture.config().join("moved"))?;
    fixture.write("lib/value.txt", "second")?;
    let second = Root::open(&fixture.config().join("lib"))?;
    let files = Confined::new(vec![first, second]);
    assert_eq!(
        files.read(&std::fs::canonicalize(file.parent().ok_or("parent")?)?.join("value.txt"))?,
        Some(b"first".to_vec())
    );
    Ok(())
}
