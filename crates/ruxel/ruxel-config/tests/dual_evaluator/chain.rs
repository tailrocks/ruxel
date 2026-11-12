//! A sibling of `dual_evaluator.rs`: the chained-placeholder sweep —
//! the malformed bodies a placeholder may not leak when its delimiters are
//! shared with its neighbours. Each variant is refused by both evaluators.

use super::*;

/// The malformed chain bodies: space, newline, tab, colon, a second
/// terminator, a three-placeholder chain of well-formed bodies, and the
/// finding's own payload — and the run-time→secret shapes whose shared
/// delimiters would let the run-time match swallow the secret's opening
/// `__`: the same body variants, an empty body and a well-formed encoding
/// included. Every one is refused as a malformed secret
/// reference under both evaluators, never left behind as literal content.
const VARIANTS: &[(&str, &str)] = &[
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
    (
        "runtime-space",
        r"P=__ruxel_rt_8d41.inventoryHost__ruxel_secret_8d41.bad body__\n",
    ),
    (
        "runtime-newline",
        r"P=__ruxel_rt_8d41.inventoryHost__ruxel_secret_8d41.bad\nbody__\n",
    ),
    (
        "runtime-tab",
        r"P=__ruxel_rt_8d41.inventoryHost__ruxel_secret_8d41.bad\tbody__\n",
    ),
    (
        "runtime-colon",
        r"P=__ruxel_rt_8d41.inventoryHost__ruxel_secret_8d41.bad:body__\n",
    ),
    (
        "runtime-empty",
        r"P=__ruxel_rt_8d41.inventoryHost__ruxel_secret_8d41.__\n",
    ),
    (
        "runtime-encoding",
        r"P=__ruxel_rt_8d41.inventoryHost__ruxel_secret_8d41.f2:AB__\n",
    ),
    (
        "unterminated-shared",
        r"P=__ruxel_secret_8d41.f2:AB__ruxel_secret_8d41.f2:CD\n",
    ),
    (
        "runtime-unterminated-shared",
        r"P=__ruxel_rt_8d41.inventoryHost__ruxel_secret_8d41.f2:CD\n",
    ),
    (
        "unterminated-body",
        r"P=__ruxel_secret_8d41.f2:AB__ruxel_secret_8d41.ruxel_secret_8d41.f2:CD\n",
    ),
];

/// Every variant, refused by both evaluators as a malformed secret
/// reference under both evaluators, never left behind as literal content.
#[test]
fn the_malformed_chain_variants_are_refused_under_both_evaluators() -> Result {
    let pkl = pkl()?;
    let modules = modules()?;
    let scratch = std::env::temp_dir().join(format!("ruxel-chain-sweep-{}", std::process::id()));
    let invalid = scratch.join("invalid");
    std::fs::create_dir_all(&invalid)?;
    std::fs::copy(root().join("inventory.pkl"), scratch.join("inventory.pkl"))?;
    for (name, content) in VARIANTS {
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
                return Err(format!("{name}: {who} accepted a malformed overlap").into());
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
