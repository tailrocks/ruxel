//! The valid corpus must survive the engine's own validation. The lowering
//! and the validator have to agree on what a valid workflow is: a corpus
//! case that lowers to a configuration the engine rejects is a bug in one
//! of the two, and only a test that runs both catches it.

use std::path::{Path, PathBuf};

fn corpus_root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/workflow"
    ))
    .to_path_buf()
}

/// The `Absent` corpus case lowers to a removal the engine accepts: a
/// removal with `recursive` is a removal of a directory with everything in
/// it, not a directory that is created.
#[test]
fn the_absent_corpus_case_passes_engine_validation() -> Result<(), Box<dyn std::error::Error>> {
    let root = corpus_root();
    let options = ruxel_config::EvalOptions {
        root_dir: Some(root.clone()),
        ..ruxel_config::EvalOptions::default()
    };
    let config = ruxel_config::eval_config(&root.join("valid").join("absent.pkl"), &options)?;
    ruxel_engine::model::validate_config(&config)?;
    Ok(())
}
