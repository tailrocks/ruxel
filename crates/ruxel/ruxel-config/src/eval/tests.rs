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
