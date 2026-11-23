use super::*;

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
    let mut evaluator = pklr::Evaluator::new();
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
    let mut evaluator = pklr::Evaluator::new();
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
