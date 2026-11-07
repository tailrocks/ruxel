use crate::common::{self, Harness};
use anyhow::{Result, ensure};
use std::{fs, path::Path};
const FIRST: &[u8] = &[0, 255, 128, 65, 10, 0, 254, 66];
const SECOND: &[u8] = &[255, 0, 1, 128, 10, 67, 253];
pub fn check(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let harness = Harness::new(root, cli, agent)?;
    let source = harness.temp.path().join("source.bin");
    let dest = harness.payload.join("copy.bin");
    fs::write(&source, FIRST)?;
    let tasks = format!(
        r#"new {{ name = "copy"; operation = new Ruxel.Copy {{ src = {}; dest = {}; mode = "0640" }} }}"#,
        common::path(&source)?,
        common::path(&dest)?
    );
    harness.write(&tasks)?;
    ensure!(
        harness.run("apply")?["changed"] == true
            && fs::read(&dest)? == FIRST
            && common::mode(&dest)? == 0o640,
        "fresh binary copy incorrect"
    );
    ensure!(
        harness.run("apply")?["changed"] == false && fs::read(&dest)? == FIRST,
        "binary copy did not converge"
    );
    fs::write(&dest, b"drift")?;
    harness.predict(true)?;
    ensure!(
        harness.run("apply")?["changed"] == true && fs::read(&dest)? == FIRST,
        "binary drift was not repaired"
    );
    fs::write(&source, SECOND)?;
    harness.predict(true)?;
    ensure!(
        harness.run("apply")?["changed"] == true && fs::read(&dest)? == SECOND,
        "binary source update ignored"
    );
    fs::write(&source, b"")?;
    harness.predict(true)?;
    ensure!(
        harness.run("apply")?["changed"] == true && fs::read(&dest)?.is_empty(),
        "empty binary source ignored"
    );
    fs::write(&source, FIRST)?;
    harness.write(&tasks.replace("name = \"copy\";", "name = \"copy\"; noLog = true;"))?;
    let private = harness.run("diff")?;
    ensure!(
        private["censored"] == true
            && private
                .get("diff")
                .is_none_or(|value| value.is_null() || value == false || value == "")
            && fs::read(&dest)?.is_empty(),
        "noLog prediction leaked a diff or mutated destination"
    );
    ensure!(
        harness.run("apply")?["censored"] == true && fs::read(&dest)? == FIRST,
        "noLog binary copy failed"
    );
    println!(
        "PASS binary copy: fresh, convergence, drift/source/empty repair, check/diff non-mutation and noLog"
    );
    Ok(())
}
