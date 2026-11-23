use crate::{
    common::{Env, quoted},
    file_helpers::{PKL, observed, snapshot, write_drift},
};
use anyhow::{Context, Result, ensure};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

pub fn run(cli: &Path, agent: &Path) -> Result<()> {
    let env = Env::new(cli, agent, "fixtures")?;
    for (name, contents) in [
        ("line", "setting=old\n"),
        ("replace", "fruit=apple\n"),
        ("block", "header\n"),
        ("absent", "remove-me\n"),
    ] {
        fs::write(env.payload.join(name), contents)?;
    }
    fs::create_dir_all(env.payload.join("tree/sub"))?;
    fs::write(env.payload.join("tree/sub/file"), "nested\n")?;
    fs::set_permissions(
        env.payload.join("tree/sub/file"),
        fs::Permissions::from_mode(0o600),
    )?;
    let mut source = PKL.to_owned();
    for name in [
        "dir", "content", "template", "link", "line", "replace", "block", "absent", "tree",
    ] {
        source = source.replace(&name.to_uppercase(), &quoted(&env.payload.join(name))?);
    }
    fs::write(&env.module, source)?;
    for (changed, phase) in [(true, "fresh"), (false, "converged")] {
        let (events, recap) = env.run("apply")?;
        ensure!(
            (recap["changed"].as_u64().context("missing changed count")? > 0) == changed,
            "{phase}: unexpected changes"
        );
        observed(&env.payload, &events)?;
    }
    write_drift(&env.payload)?;
    for mode in ["check", "diff"] {
        let before = snapshot(&env.payload)?;
        let (events, recap) = env.run(mode)?;
        ensure!(
            recap["changed"].as_u64().context("missing changed count")? > 0
                && snapshot(&env.payload)? == before,
            "{mode}: mutation or missing prediction"
        );
        if mode == "diff" {
            ensure!(
                events.iter().any(|event| event["result"]["diff"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty())),
                "diff missing"
            );
        }
    }
    let (events, recap) = env.run("apply")?;
    ensure!(
        recap["changed"].as_u64().context("missing changed count")? > 0,
        "repair reported no change"
    );
    observed(&env.payload, &events)?;
    println!("PASS file lifecycle: fresh, convergence, check/diff and drift repair");
    Ok(())
}
