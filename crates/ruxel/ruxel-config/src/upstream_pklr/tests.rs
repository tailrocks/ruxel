use super::{eval_confined, expand_glob};
use crate::eval::EvalOptions;
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A scratch directory outside every confined root.
fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("ruxel-upstream-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("scratch directory");
    directory
}

#[test]
fn upstream_expands_globs() -> Result<(), Box<dyn std::error::Error>> {
    let expanded = expand_glob(&manifest_dir(), "*.toml")?;
    assert!(
        expanded.iter().any(|path| path.ends_with("Cargo.toml")),
        "{expanded:?}"
    );
    Ok(())
}

#[test]
fn the_upstream_path_evaluates_a_module_inside_the_root() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = scratch("inside");
    let module = directory.join("main.pkl");
    std::fs::write(&module, "value = \"evaluated\"\n")?;
    let value = eval_confined(&module, &EvalOptions::default())?;
    assert_eq!(value, serde_json::json!({"value": "evaluated"}));
    Ok(())
}

#[test]
fn the_upstream_path_refuses_a_read_outside_the_root() -> Result<(), Box<dyn std::error::Error>> {
    let directory = scratch("outside");
    let outside = directory.join("outside.txt");
    std::fs::write(&outside, "outside")?;
    let root = scratch("root");
    let module = root.join("main.pkl");
    std::fs::write(
        &module,
        format!("value = read(\"file:{}\")\n", outside.display()),
    )?;
    let error = eval_confined(&module, &EvalOptions::default());
    let message = match error {
        Ok(value) => panic!("the read outside the root succeeded: {value}"),
        Err(error) => error.to_string(),
    };
    // Ruxel's own refusal, from the confined capabilities the corpus uses.
    assert!(
        message.contains("outside the configuration root"),
        "{message}"
    );
    Ok(())
}

#[test]
fn the_upstream_path_searches_the_module_path() -> Result<(), Box<dyn std::error::Error>> {
    let directory = scratch("module-path");
    std::fs::write(directory.join("served.pkl"), "value = \"served\"")?;
    let root = scratch("module-path-root");
    let module = root.join("main.pkl");
    std::fs::write(
        &module,
        "import \"modulepath:/served.pkl\"\nvalue = served.value\n",
    )?;
    let options = EvalOptions {
        module_path: vec![directory],
        root_dir: None,
    };
    let value = eval_confined(&module, &options)?;
    assert_eq!(value, serde_json::json!({"value": "served"}));
    Ok(())
}

#[test]
fn the_upstream_path_refuses_an_unknown_module_path_entry() {
    let root = scratch("module-path-missing");
    let module = root.join("main.pkl");
    std::fs::write(
        &module,
        "import \"modulepath:/absent.pkl\"\nvalue = absent.value\n",
    )
    .expect("module");
    let missing = Path::new("/nonexistent-ruxel-module-path");
    let options = EvalOptions {
        module_path: vec![missing.to_path_buf()],
        root_dir: None,
    };
    let error = eval_confined(&module, &options);
    assert!(error.is_err());
}

#[test]
fn every_gap_cites_an_upstream_file_and_line_and_a_grep_result() {
    for gap in super::CAPABILITY_GAPS {
        let (file, line) = gap
            .location
            .rsplit_once(':')
            .unwrap_or_else(|| panic!("{}: no line number in {}", gap.family, gap.location));
        assert!(
            file.starts_with("src/"),
            "{}: {} is not an upstream path",
            gap.family,
            file
        );
        assert!(
            !line.is_empty() && line.bytes().all(|byte| byte.is_ascii_digit()),
            "{}: {} does not end in a line number",
            gap.family,
            gap.location
        );
        assert!(
            gap.absent.contains("0 hits"),
            "{}: {} does not record a grep result",
            gap.family,
            gap.absent
        );
    }
}
