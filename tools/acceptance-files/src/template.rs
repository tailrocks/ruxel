use crate::common::{Env, quoted, task};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::MetadataExt, path::Path};

const PIN: &str = "c231e95be0d13fb69bd1908c94143408011e6ea29219608325711b16ecd163bb";
const PKL: &str = r#"import "modulepath:/ruxel/engine/Engine.pkl" as Ruxel
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "native template"; targets { "all" }; vars {
  ["fixture_lines"] = new Ruxel.ListExpr { items { new Ruxel.Literal { value = "alpha" }; new Ruxel.Literal { value = "beta" } } }
  ["fixture_optional"] = new Ruxel.Literal { value = "" }
  ["fixture_enabled"] = new Ruxel.Literal { value = ENABLED }
 }; tasks { new { name = "template"; operation = new Ruxel.Template { src = SOURCE; dest = DEST; mode = "0640" } } } } }
} }
"#;
fn write_module(env: &Env, source: &Path, dest: &Path, enabled: bool) -> Result<()> {
    let pkl = PKL
        .replace("ENABLED", if enabled { "true" } else { "false" })
        .replace("SOURCE", &quoted(source)?)
        .replace("DEST", &quoted(dest)?);
    fs::write(&env.module, pkl)?;
    Ok(())
}
fn digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn expected(enabled: bool, changed: bool) -> String {
    let digest = digest(b"synthetic");
    let mut result = format!(
        "alpha\nbeta\njoined=alpha,beta\nhash={digest}\noptional=fallback\nencoded=synthetic%20value\n"
    );
    if enabled {
        result.push_str(if changed {
            "conditional=changed\n"
        } else {
            "conditional=enabled\n"
        });
    }
    result
}
fn apply(env: &Env, dest: &Path, enabled: bool, source_changed: bool, changed: bool) -> Result<()> {
    let (events, _) = env.run("apply")?;
    ensure!(
        task(&events, "template")?["changed"] == changed
            && fs::read(dest)? == expected(enabled, source_changed).as_bytes(),
        "template apply mismatch"
    );
    Ok(())
}
fn predict(env: &Env, dest: &Path) -> Result<()> {
    let before = (fs::read(dest)?, fs::metadata(dest)?.mode() & 0o7777);
    for mode in ["check", "diff"] {
        let (events, _) = env.run(mode)?;
        let result = task(&events, "template")?;
        ensure!(
            result["changed"] == true
                && (fs::read(dest)?, fs::metadata(dest)?.mode() & 0o7777) == before,
            "{mode}: prediction or nonmutation mismatch"
        );
        if mode == "diff" {
            ensure!(
                result["diff"].as_str().is_some_and(|v| !v.is_empty()),
                "template diff missing"
            );
        }
    }
    Ok(())
}
pub fn run(cli: &Path, agent: &Path, asset: &Path) -> Result<()> {
    let bytes = fs::read(asset)?;
    ensure!(digest(&bytes) == PIN, "template asset pin mismatch");
    let env = Env::new(cli, agent, "testdata/workloads/pr66")?;
    let source = env
        .module
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing parent"))?
        .join("assets/template.pkl");
    fs::create_dir_all(
        source
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing parent"))?,
    )?;
    ensure!(
        std::str::from_utf8(&bytes)?
            .matches("import \"modulepath:/ruxel/engine/Engine.pkl\"")
            .count()
            == 1,
        "unexpected template import"
    );
    fs::write(&source, &bytes)?;
    let dest = env.payload.join("rendered");
    write_module(&env, &source, &dest, true)?;
    apply(&env, &dest, true, false, true)?;
    ensure!(
        fs::metadata(&dest)?.mode() & 0o7777 == 0o640,
        "template mode mismatch"
    );
    apply(&env, &dest, true, false, false)?;
    fs::write(&dest, "drift")?;
    predict(&env, &dest)?;
    apply(&env, &dest, true, false, true)?;
    let original = fs::read_to_string(&source)?;
    ensure!(
        original.matches("conditional=enabled").count() == 1,
        "unexpected template source"
    );
    fs::write(
        &source,
        original.replace("conditional=enabled", "conditional=changed"),
    )?;
    predict(&env, &dest)?;
    apply(&env, &dest, true, true, true)?;
    write_module(&env, &source, &dest, false)?;
    predict(&env, &dest)?;
    apply(&env, &dest, false, false, true)?;
    println!("PASS template source: render, convergence, drift/source/scope changes, check/diff");
    Ok(())
}
