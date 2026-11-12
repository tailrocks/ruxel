//! The long `join` separator secret under both evaluators: a sibling of
//! `dual_evaluator.rs` within the one dual-evaluator target, the repo
//! `tests/schema.rs` + `tests/schema/` split pattern.

use super::*;

/// A `join` separator of a million characters is a seven-digit length
/// prefix: the encoder writes it, so the decoder must read it. The
/// workflow is generated, because a million characters do not belong in a
/// committed file.
///
/// The secret is an environment value, not file content: the content of a
/// file is scanned for placeholders, and pklr's regular expressions do not
/// walk a million characters.
#[test]
fn a_long_join_separator_decodes_under_both_evaluators() -> Result {
    let pkl = pkl()?;
    let modules = modules()?;
    let directory = std::env::temp_dir().join(format!(
        "ruxel-dual-evaluator-long-{}-{}",
        std::process::id(),
        std::process::id()
    ));
    std::fs::create_dir_all(&directory)?;
    std::fs::copy(
        root().join("inventory.pkl"),
        directory.join("inventory.pkl"),
    )?;
    let separator = "x".repeat(1_000_000);
    let workflow = format!(
        "amends \"modulepath:/ruxel/Workflow.pkl\"\n\
         import \"modulepath:/ruxel/Core.pkl\" as core\n\
         import \"inventory.pkl\"\n\
         \n\
         stages {{ new Stage {{ title = \"t\"; hosts = inventory.workers; steps {{\n\
         \x20 new Command {{\n\
         \x20   run {{ \"/bin/true\" }}\n\
         \x20   environment {{ [\"BIG\"] = core.fnox(\"APP_SECRET\").join(\"{separator}\", core.fnox(\"APP_SECRET_TWO\")) }}\n\
         \x20 }}\n\
         }} }} }}\n"
    );
    let path = directory.join("long-separator.pkl");
    std::fs::write(&path, workflow)?;
    compare(
        "long-separator.pkl",
        &path,
        &directory,
        &pkl,
        &modules,
        true,
    )?;
    drop(std::fs::remove_dir_all(&directory));
    Ok(())
}
