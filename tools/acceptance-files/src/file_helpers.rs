use crate::common::task;
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub(super) const PKL: &str = r#"import "../schema/Ruxel.pkl"
output { value = new Ruxel.Config {
 inventory { hosts { ["fixture"] { address = "127.0.0.1" } }; groups { ["all"] { hosts { "fixture" } } } }
 plays { new { name = "files"; targets { "all" }; vars { ["value"] = new Ruxel.Literal { value = "rendered" } }; tasks {
  new { name = "remove"; operation = new Ruxel.File { path = ABSENT; state = "absent" } }
  new { name = "recurse"; operation = new Ruxel.File { path = TREE; state = "directory"; mode = "0700"; recurse = true } }
  new { name = "directory"; operation = new Ruxel.File { path = DIR; state = "directory"; mode = "0700" } }
  new { name = "content"; operation = new Ruxel.Copy { dest = CONTENT; content = "synthetic-content\n"; mode = "0640" } }
  new { name = "template"; operation = new Ruxel.Template { dest = TEMPLATE; content = new Ruxel.Call { `function` = "concat"; args { new Ruxel.Literal { value = "prefix-" }; new Ruxel.Ref { path { "vars"; "value" } } } } } }
  new { name = "link"; operation = new Ruxel.File { path = LINK; src = CONTENT; state = "link" } }
  new { name = "line"; operation = new Ruxel.LineInFile { path = LINE; regexp = "^setting="; line = "setting=new"; state = "present" } }
  new { name = "replace"; operation = new Ruxel.Replace { path = REPLACE; regexp = "apple"; replace = "pear" } }
  new { name = "block"; operation = new Ruxel.BlockInFile { path = BLOCK; block = "alpha\nbeta" } }
  new { name = "stat"; operation = new Ruxel.Stat { path = CONTENT } }
  new { name = "slurp"; operation = new Ruxel.Slurp { src = CONTENT } }
 } } }
} }
"#;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Directory(u32),
    File(u32, Vec<u8>),
    Link(PathBuf),
}
fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Entry>) -> Result<()> {
    for item in fs::read_dir(dir)? {
        let path = item?.path();
        let metadata = fs::symlink_metadata(&path)?;
        let key = path.strip_prefix(root)?.to_path_buf();
        let entry = if metadata.file_type().is_symlink() {
            Entry::Link(fs::read_link(&path)?)
        } else if metadata.is_dir() {
            walk(root, &path, out)?;
            Entry::Directory(metadata.mode() & 0o7777)
        } else if metadata.is_file() {
            Entry::File(metadata.mode() & 0o7777, fs::read(&path)?)
        } else {
            anyhow::bail!("unexpected fixture entry")
        };
        out.insert(key, entry);
    }
    Ok(())
}
pub(super) fn snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Entry>> {
    let mut result = BTreeMap::new();
    walk(root, root, &mut result)?;
    Ok(result)
}
pub(super) fn write_drift(payload: &Path) -> Result<()> {
    for (name, content) in [
        ("content", "drift\n"),
        ("template", "drift"),
        ("absent", "remove-again\n"),
        ("line", "setting=old\n"),
        ("replace", "fruit=apple\n"),
        ("block", "header\n"),
    ] {
        fs::write(payload.join(name), content)?;
    }
    fs::set_permissions(
        payload.join("tree/sub/file"),
        fs::Permissions::from_mode(0o600),
    )?;
    Ok(())
}
pub(super) fn observed(payload: &Path, events: &[Value]) -> Result<()> {
    ensure!(!payload.join("absent").exists(), "absent path still exists");
    for name in ["tree", "tree/sub", "tree/sub/file", "dir"] {
        ensure!(
            fs::metadata(payload.join(name))?.mode() & 0o7777 == 0o700,
            "mode incorrect: {name}"
        );
    }
    ensure!(payload.join("dir").is_dir(), "directory missing");
    for (name, expected) in [
        ("content", "synthetic-content\n"),
        ("template", "prefix-rendered"),
        ("line", "setting=new\n"),
        ("replace", "fruit=pear\n"),
        (
            "block",
            "header\n# BEGIN ANSIBLE MANAGED BLOCK\nalpha\nbeta\n# END ANSIBLE MANAGED BLOCK\n",
        ),
    ] {
        ensure!(
            fs::read(payload.join(name))? == expected.as_bytes(),
            "content incorrect: {name}"
        );
    }
    ensure!(
        fs::metadata(payload.join("content"))?.mode() & 0o7777 == 0o640,
        "copy mode incorrect"
    );
    let link = payload.join("link");
    ensure!(
        fs::symlink_metadata(&link)?.file_type().is_symlink()
            && fs::canonicalize(&link)? == fs::canonicalize(payload.join("content"))?,
        "link incorrect"
    );
    ensure!(
        task(events, "stat")?["stat"]["exists"] == true,
        "stat incorrect"
    );
    ensure!(
        STANDARD.decode(
            task(events, "slurp")?["content"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("slurp content missing"))?
        )? == b"synthetic-content\n",
        "slurp incorrect"
    );
    Ok(())
}

#[cfg(test)]
#[path = "file_helpers_tests.rs"]
mod tests;
