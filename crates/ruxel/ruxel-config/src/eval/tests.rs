use super::eval;
use std::path::Path;

#[test]
fn minimal_evaluates() {
    assert!(
        eval(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/eval/minimal.pkl"
        )))
        .is_ok()
    );
}

#[test]
fn environment_reads_are_denied() {
    assert!(
        eval(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/eval/env.pkl"
        )))
        .is_err()
    );
}

#[test]
fn network_imports_are_denied() {
    assert!(
        eval(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/eval/network.pkl"
        )))
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn local_resources_and_sibling_module_imports_work() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let dir =
        std::env::temp_dir().join(format!("ruxel-pkl-local-resources-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("config"))?;
    std::fs::write(dir.join("schema.pkl"), "value = \"schema-marker\"\n")?;
    let resource = dir.join("config/input.txt");
    std::fs::write(&resource, "local-marker")?;
    symlink(resource.canonicalize()?, dir.join("config/link.txt"))?;
    let module = dir.join("config/main.pkl");
    std::fs::write(
        &module,
        "import \"../schema.pkl\" as schema\nmoduleValue = schema.value\nlocalValue = read(\"input.txt\").text\nlinked = read(\"link.txt\").text\nall = read*(\"*.txt\")\n",
    )?;
    let result = eval(&module)?;
    assert_eq!(result["moduleValue"], "schema-marker");
    assert_eq!(result["localValue"], "local-marker");
    assert_eq!(result["linked"], "local-marker");
    assert_eq!(result["all"]["input.txt"]["text"], "local-marker");
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn resources_cannot_escape_via_paths_or_symlinks() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let dir = std::env::temp_dir().join(format!("ruxel-pkl-escape-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("config"))?;
    std::fs::write(dir.join("outside.txt"), "outside-sentinel")?;
    symlink(
        dir.join("outside.txt").canonicalize()?,
        dir.join("config/escape.txt"),
    )?;
    symlink(dir.canonicalize()?, dir.join("config/escape-dir"))?;
    let module = dir.join("config/main.pkl");
    for expression in [
        "read(\"../outside.txt\").text",
        "read(\"escape.txt\").text",
        "read(\"escape-dir/outside.txt\").text",
        "read*(\"escape-dir/*.txt\")",
    ] {
        std::fs::write(&module, format!("value = {expression}\n"))?;
        assert!(
            eval(&module).is_err(),
            "resource escape accepted: {expression}"
        );
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn resource_symlink_race_never_reads_outside() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let dir = std::env::temp_dir().join(format!("ruxel-pkl-race-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("config"))?;
    std::fs::write(dir.join("config/inside.txt"), "inside")?;
    std::fs::write(dir.join("outside.txt"), "outside-sentinel")?;
    let inside = dir.join("config/inside.txt").canonicalize()?;
    let outside = dir.join("outside.txt").canonicalize()?;
    let link = dir.join("config/link.txt");
    symlink(&inside, &link)?;
    let module = dir.join("config/main.pkl");
    std::fs::write(&module, "value = read(\"link.txt\").text\n")?;
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker = std::thread::spawn(move || {
        let replacement = link.with_extension("replacement");
        while !worker_stop.load(Ordering::Relaxed) {
            for target in [&outside, &inside] {
                if symlink(target, &replacement).is_ok() {
                    let _ = std::fs::rename(&replacement, &link);
                }
            }
        }
    });
    let mut escaped = false;
    for _ in 0..200 {
        if let Ok(result) = eval(&module) {
            escaped |= result["value"] == "outside-sentinel";
        }
    }
    stop.store(true, Ordering::Relaxed);
    worker.join().map_err(|_| "race worker panicked")?;
    std::fs::remove_dir_all(dir)?;
    assert!(!escaped, "resource race read outside sentinel");
    Ok(())
}

#[cfg(unix)]
#[test]
fn module_symlinks_set_the_resolved_resource_root() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let dir = std::env::temp_dir().join(format!("ruxel-module-link-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("actual"))?;
    std::fs::create_dir_all(dir.join("entry"))?;
    std::fs::write(dir.join("actual/value.txt"), "resolved")?;
    std::fs::write(
        dir.join("actual/main.pkl"),
        "value = read(\"value.txt\").text\n",
    )?;
    symlink(
        dir.join("actual/main.pkl").canonicalize()?,
        dir.join("entry/main.pkl"),
    )?;
    assert_eq!(eval(&dir.join("entry/main.pkl"))?["value"], "resolved");
    std::fs::remove_dir_all(dir)?;
    Ok(())
}
