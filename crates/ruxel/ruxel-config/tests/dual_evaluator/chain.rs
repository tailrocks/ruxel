//! A sibling of `dual_evaluator.rs`: the chained-placeholder sweep —
//! the malformed bodies a placeholder may not leak when its delimiters are
//! shared with its neighbours. Each variant is refused by both evaluators.

use super::*;

/// The malformed chain bodies: space, newline, tab, colon, a second
/// terminator, a three-placeholder chain of well-formed bodies, and the
/// finding's own payload. Every one is refused as a malformed secret
/// reference under both evaluators, never left behind as literal content.
#[test]
fn the_malformed_chain_variants_are_refused_under_both_evaluators() -> Result {
    let pkl = pkl()?;
    let modules = modules()?;
    let scratch = std::env::temp_dir().join(format!("ruxel-chain-sweep-{}", std::process::id()));
    let invalid = scratch.join("invalid");
    std::fs::create_dir_all(&invalid)?;
    std::fs::copy(root().join("inventory.pkl"), scratch.join("inventory.pkl"))?;
    let variants: &[(&str, &str)] = &[
        (
            "space",
            r"P=__ruxel_secret_8d41.f2:AB__ruxel_secret_8d41.bad body__ruxel_secret_8d41.f2:ZQ__\n",
        ),
        (
            "newline",
            r"P=__ruxel_secret_8d41.f2:AB__ruxel_secret_8d41.bad\nbody__ruxel_secret_8d41.f2:ZQ__\n",
        ),
        (
            "tab",
            r"P=__ruxel_secret_8d41.f2:AB__ruxel_secret_8d41.bad\tbody__ruxel_secret_8d41.f2:ZQ__\n",
        ),
        (
            "colon",
            r"P=__ruxel_secret_8d41.f2:AB__ruxel_secret_8d41.bad:body__ruxel_secret_8d41.f2:ZQ__\n",
        ),
        (
            "second-terminator",
            r"P=__ruxel_secret_8d41.f2:AB__ruxel_secret_8d41.bad__body__ruxel_secret_8d41.f2:ZQ__\n",
        ),
        (
            "chain-three",
            r"P=__ruxel_secret_8d41.f2:AB__ruxel_secret_8d41.f2:XA__ruxel_secret_8d41.f2:ZQ__\n",
        ),
        (
            "chain-finding",
            r"P=__ruxel_secret_8d41.f2:AB__ruxel_secret_8d41.f1:X__ruxel_secret_8d41.f1:Z__\n",
        ),
    ];
    for (name, content) in variants {
        let path = invalid.join(format!("{name}.pkl"));
        std::fs::write(
            &path,
            format!(
                "amends \"modulepath:/ruxel/Workflow.pkl\"\n\
                 import \"../inventory.pkl\"\n\n\
                 stages {{ new Stage {{ title = \"t\"; hosts = inventory.workers; steps {{\n  \
                 new File {{\n    path = \"/etc/app/app.env\"\n    mode = \"0600\"\n    \
                 content = \"{content}\"\n  }}\n}} }} }}\n"
            ),
        )?;
        let under_pklr = under_pklr(&path, &scratch);
        let under_official = under_pkl(&pkl, &modules, &path, &scratch);
        for (who, answer) in [("pklr", under_pklr), ("the official pkl", under_official)] {
            if !answer.failed() {
                return Err(format!("{name}: {who} accepted a malformed chain").into());
            }
            if !answer
                .error
                .unwrap_or_default()
                .contains("File content contains a malformed secret reference")
            {
                return Err(format!("{name}: {who} refused it with another sentence").into());
            }
        }
    }
    drop(std::fs::remove_dir_all(&scratch));
    Ok(())
}
