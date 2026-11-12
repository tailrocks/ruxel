use super::{eval_source_json, expand_glob};
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
fn upstream_evaluates_a_source() -> Result<(), Box<dyn std::error::Error>> {
    let path = manifest_dir().join("testdata/eval/entry.pkl");
    let value = eval_source_json("value = 1", &path, &[])?;
    assert_eq!(value, serde_json::json!({"value": 1}));
    Ok(())
}

#[test]
fn upstream_searches_the_module_path() -> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::env::temp_dir().join(format!("ruxel-upstream-pklr-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    std::fs::write(directory.join("served.pkl"), "value = \"served\"")?;
    let path = manifest_dir().join("testdata/eval/entry.pkl");
    let value = eval_source_json(
        "import \"modulepath:/served.pkl\"\nvalue = served.value",
        &path,
        &[directory],
    )?;
    assert_eq!(value, serde_json::json!({"value": "served"}));
    Ok(())
}

#[test]
fn upstream_refuses_an_unknown_module_path_entry() {
    let path = manifest_dir().join("testdata/eval/entry.pkl");
    let missing = Path::new("/nonexistent-ruxel-module-path");
    let error = eval_source_json(
        "import \"modulepath:/absent.pkl\"\nvalue = absent.value",
        &path,
        &[missing.to_path_buf()],
    );
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
