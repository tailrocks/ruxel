use crate::common::{self, Harness};
use anyhow::{Result, ensure};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
pub fn check(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let harness = Harness::new(root, cli, agent)?;
    let log = harness.temp.path().join("actions.log");
    let helper = harness.temp.path().join("action");
    fs::write(
        &helper,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" >> {}\n",
            common::shell(&log)?
        ),
    )?;
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))?;
    let items: Vec<_> = (1..=36).map(|i| format!("{i:02}")).collect();
    let values = items
        .iter()
        .map(|item| {
            Ok(format!(
                "new Ruxel.Literal {{ value = {} }}",
                common::pkl(item)?
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join("; ");
    let prefix = common::pkl(&format!("{} ", common::shell(&helper)?))?;
    harness.write(&format!(r#"new {{ name = "action"; `loop` = new Ruxel.ListExpr {{ items {{ {values} }} }};
 operation = new Ruxel.Shell {{ cmd = new Ruxel.Call {{ `function` = "concat"; args {{ new Ruxel.Literal {{ value = {prefix} }}; new Ruxel.Ref {{ path {{ "item" }} }} }} }} }} }}"#))?;
    harness.events("apply", 30)?;
    ensure!(
        fs::read_to_string(&log)?.lines().collect::<Vec<_>>() == items,
        "fresh apply did not run 36 ordered actions"
    );
    harness.events("apply", 30)?;
    ensure!(
        fs::read_to_string(&log)?.lines().collect::<Vec<_>>()
            == items
                .iter()
                .chain(&items)
                .map(String::as_str)
                .collect::<Vec<_>>(),
        "rerun did not repeat all 36 ordered actions"
    );
    for mode in ["check", "diff"] {
        let before = fs::read(&log)?;
        harness.events(mode, 30)?;
        ensure!(fs::read(&log)? == before, "{mode} executed an action");
    }
    println!("PASS repeated actions: 36 ordered actions on each apply, check/diff non-mutation");
    Ok(())
}
