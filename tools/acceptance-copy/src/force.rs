use crate::common::{self, Harness};
use anyhow::{Result, ensure};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
pub fn check(root: &Path, cli: &Path, agent: &Path) -> Result<()> {
    let harness = Harness::new(root, cli, agent)?;
    let source = harness.temp.path().join("source");
    let dest = harness.payload.join("destination");
    fs::write(&source, b"original\x00\xff")?;
    harness.write(&format!(r#"new {{ name = "copy"; operation = new Ruxel.Copy {{ src = {}; dest = {}; force = false; mode = "0640" }} }}"#,common::path(&source)?,common::path(&dest)?))?;
    ensure!(
        harness.run("apply")?["changed"] == true
            && fs::read(&dest)? == b"original\x00\xff"
            && common::mode(&dest)? == 0o640,
        "force=false fresh creation incorrect"
    );
    ensure!(
        harness.run("apply")?["changed"] == false,
        "initial force=false rerun did not converge"
    );
    fs::write(&dest, b"local-content")?;
    fs::set_permissions(&dest, fs::Permissions::from_mode(0o600))?;
    harness.predict(true)?;
    ensure!(
        harness.run("apply")?["changed"] == true
            && fs::read(&dest)? == b"local-content"
            && common::mode(&dest)? == 0o640,
        "metadata repair overwrote content or failed"
    );
    ensure!(
        harness.run("apply")?["changed"] == false,
        "metadata rerun did not converge"
    );
    fs::write(&source, b"new-source\x80")?;
    harness.predict(false)?;
    ensure!(
        harness.run("apply")?["changed"] == false && fs::read(&dest)? == b"local-content",
        "force=false overwrote an existing destination"
    );
    fs::remove_file(&dest)?;
    harness.predict(true)?;
    ensure!(
        harness.run("apply")?["changed"] == true
            && fs::read(&dest)? == b"new-source\x80"
            && common::mode(&dest)? == 0o640,
        "missing destination was not recreated"
    );
    println!(
        "PASS copy force=false: create, metadata repair, preserve content, recreate, check/diff"
    );
    Ok(())
}
