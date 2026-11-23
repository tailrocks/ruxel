use super::{Fixture, Result, refused};
use crate::{EvalOptions, eval};
use serde_json::json;
use std::sync::atomic::Ordering;

#[test]
fn symlinks_inside_the_root_are_refused() -> Result {
    let fixture = Fixture::new()?;
    std::os::unix::fs::symlink(
        fixture.0.join("outside.txt"),
        fixture.config().join("file.txt"),
    )?;
    std::os::unix::fs::symlink(&fixture.0, fixture.config().join("dir"))?;
    std::os::unix::fs::symlink(
        fixture.0.join("outside.pkl"),
        fixture.config().join("module.pkl"),
    )?;
    for source in [
        "value = read(\"file.txt\").text",
        "value = read(\"dir/outside.txt\").text",
        "import \"module.pkl\" as m\nvalue = m.value",
        "import \"dir/outside.pkl\" as m\nvalue = m.value",
    ] {
        assert!(refused(fixture.eval(source)), "{source}");
    }
    Ok(())
}

#[test]
fn racing_resource_symlinks_never_supply_outside_bytes() -> Result {
    use std::sync::{Arc, atomic::AtomicBool};
    let fixture = Fixture::new()?;
    let resource = fixture.write("resource.txt", "inside")?;
    let root = fixture.write("main.pkl", "value = read(\"resource.txt\").text")?;
    let outside = fixture.0.join("outside.txt");
    let held = fixture.config().join("held.txt");
    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = Arc::clone(&stop);
    let writer = std::thread::spawn(move || {
        while !writer_stop.load(Ordering::Relaxed) {
            if std::fs::rename(&resource, &held).is_ok() {
                drop(std::os::unix::fs::symlink(&outside, &resource));
                drop(std::fs::remove_file(&resource));
                drop(std::fs::rename(&held, &resource));
            }
        }
    });
    let results = (0..100)
        .map(|_| eval(&root, &EvalOptions::default()))
        .collect::<Vec<_>>();
    stop.store(true, Ordering::Relaxed);
    assert!(writer.join().is_ok());
    for value in results.into_iter().flatten() {
        assert_eq!(value, json!({"value": "inside"}));
    }
    Ok(())
}

#[test]
fn absolute_paths_match_only_the_given_or_canonical_root_name() -> Result {
    let fixture = Fixture::new()?;
    fixture.write("inside.txt", "inside")?;
    // The spelling the root was given (on macOS, /var/... for /private/var/...).
    let given = format!(
        "value = read(\"file:{}\").text",
        fixture.config().join("inside.txt").display()
    );
    assert_eq!(fixture.eval(&given)?, json!({"value": "inside"}));
    let canonical = format!(
        "value = read(\"file:{}\").text",
        std::fs::canonicalize(fixture.config())?
            .join("inside.txt")
            .display()
    );
    assert_eq!(fixture.eval(&canonical)?, json!({"value": "inside"}));
    // Any other alias is refused without being resolved.
    let alias = fixture.0.with_extension("alias");
    std::os::unix::fs::symlink(&fixture.0, &alias)?;
    let other = format!(
        "value = read(\"file:{}\").text",
        alias.join("config/inside.txt").display()
    );
    let result = fixture.eval(&other);
    std::fs::remove_file(&alias)?;
    assert!(refused(result));
    Ok(())
}

#[test]
fn symlinked_root_module_is_refused_not_followed() -> Result {
    let fixture = Fixture::new()?;
    std::fs::write(
        fixture.0.join("target.pkl"),
        "value = read(\"outside.txt\").text",
    )?;
    let link = fixture.config().join("main.pkl");
    std::os::unix::fs::symlink(fixture.0.join("target.pkl"), &link)?;
    assert!(refused(
        eval(&link, &EvalOptions::default()).map_err(|error| error.to_string())
    ));
    Ok(())
}
