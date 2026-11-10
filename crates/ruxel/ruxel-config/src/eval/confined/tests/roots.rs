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

#[cfg(unix)]
#[test]
fn root_spellings_with_parent_segments_keep_their_alias() -> Result {
    let fixture = Fixture::new()?;
    fixture.write("sub/keep", "")?;
    let alias = fixture.0.with_extension("alias");
    std::os::unix::fs::symlink(&fixture.0, &alias)?;
    let options = EvalOptions {
        root_dir: Some(alias.join("config/sub/..")),
        ..EvalOptions::default()
    };
    let root = fixture.write("main.pkl", "value = 1")?;
    let result = eval(&alias.join("config/main.pkl"), &options);
    std::fs::remove_file(&alias)?;
    assert_eq!(result?, json!({"value": 1}));
    assert!(root.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn execute_only_directories_still_resolve_readable_modules() -> Result {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new()?;
    fixture.write("roles/role.pkl", "value = \"role\"")?;
    let root = fixture.write(
        "main.pkl",
        "import \"roles/role.pkl\" as r\nvalue = r.value",
    )?;
    let execute_only = std::fs::Permissions::from_mode(0o111);
    std::fs::set_permissions(fixture.config().join("roles"), execute_only.clone())?;
    std::fs::set_permissions(fixture.config(), execute_only)?;
    let result = eval(&root, &EvalOptions::default());
    for directory in [fixture.config(), fixture.config().join("roles")] {
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755))?;
    }
    assert_eq!(result?, json!({"value": "role"}));
    Ok(())
}
