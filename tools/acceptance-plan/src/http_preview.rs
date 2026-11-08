use crate::{pins, process};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    net::TcpListener,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Duration,
};

const TASKS: [&str; 6] = [
    "Create synthetic system fixture root",
    "Create public synthetic payload source",
    "Start disposable payload server",
    "Wait for disposable payload server",
    "Download public synthetic payload",
    "Stop disposable payload server",
];

fn safe_root(root: &Path) -> Result<&str> {
    let root = root.to_str().context("non-UTF8 fixture path")?;
    ensure!(
        root.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/._-".contains(&c)),
        "unsafe fixture path"
    );
    Ok(root)
}

fn subset(source: &str, root: &Path, port: u16) -> Result<String> {
    let prefix_end = source
        .find("    tasks {\n")
        .context("missing fixture tasks")?
        + "    tasks {\n".len();
    let mut output =
        source[..prefix_end].replace("../../../schema/Ruxel.pkl", "../schema/Ruxel.pkl");
    for name in TASKS {
        let marker = format!("      new {{\n        name = \"{name}\"\n");
        ensure!(
            source.matches(&marker).count() == 1,
            "missing or duplicate fixture task {name}"
        );
        let start = source.find(&marker).context("missing task")?;
        let end = source[start + marker.len()..]
            .find("\n      }")
            .context("unterminated fixture task")?
            + start
            + marker.len()
            + "\n      }".len();
        output.push_str(&source[start..end]);
        output.push('\n');
    }
    output.push_str("      new {\n        name = \"Verify payload server stopped\"\n        changedWhen = literal(false)\n        operation = new Ruxel.Shell { cmd = \"for attempt in 1 2 3 4 5 6 7 8 9 10; do\\n  state=$(ps -o stat= -p $(cat __RUN_ROOT__/http.pid) 2>/dev/null || true); case x$state in xZ*|x) if ! /usr/bin/curl --fail --silent --max-time 1 http://127.0.0.1:18080/payload-source.txt >/dev/null; then exit 0; fi;; esac\\n  sleep 0.1\\ndone\\nexit 1\" }\n      }\n    }\n  } }\n} }\n");
    let root = safe_root(root)?;
    Ok(output
        .replace("__RUN_ROOT__", root)
        .replace("18080", &port.to_string()))
}

type Snapshot = BTreeMap<PathBuf, (u32, u64, i64, i64, Vec<u8>)>;

fn snapshot(root: &Path) -> Result<Snapshot> {
    fn walk(base: &Path, dir: &Path, entries: &mut Snapshot) -> Result<()> {
        for item in fs::read_dir(dir)? {
            let path = item?.path();
            let meta = fs::symlink_metadata(&path)?;
            ensure!(!meta.file_type().is_symlink(), "unexpected fixture symlink");
            let bytes = if meta.is_dir() {
                walk(base, &path, entries)?;
                Vec::new()
            } else {
                ensure!(meta.is_file(), "unexpected fixture entry");
                fs::read(&path)?
            };
            entries.insert(
                path.strip_prefix(base)?.to_owned(),
                (
                    meta.mode(),
                    meta.ino(),
                    meta.mtime(),
                    meta.mtime_nsec(),
                    bytes,
                ),
            );
        }
        Ok(())
    }
    let mut entries = BTreeMap::new();
    walk(root, root, &mut entries)?;
    Ok(entries)
}

fn result<'a>(events: &'a [Value], name: &str) -> Result<&'a Value> {
    let mut results = events.iter().filter(|v| v["task"] == name);
    let event = results.next().context("missing HTTP task")?;
    ensure!(
        results.next().is_none() && event["result"]["failed"] == false,
        "invalid HTTP task {name}"
    );
    Ok(&event["result"])
}

pub fn run(root: &Path, cli: &Path, agent: &Path, http: &Path) -> Result<()> {
    let verified = pins::verified(root)?;
    let source = std::str::from_utf8(
        verified
            .get(Path::new("testdata/workloads/pr66/system-surface.pkl"))
            .context("missing pinned system fixture")?,
    )?;
    let temp = tempfile::tempdir()?;
    let schema = temp.path().join("schema");
    let workload = temp.path().join("fixture/workload.pkl");
    let payload = temp.path().join("payload");
    fs::create_dir(&schema)?;
    fs::create_dir_all(workload.parent().context("missing fixture directory")?)?;
    fs::create_dir(&payload)?;
    fs::copy(root.join("schema/Ruxel.pkl"), schema.join("Ruxel.pkl"))?;
    fs::copy(http, payload.join("ruxel-http-fixture"))?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    fs::write(&workload, subset(source, &payload, address.port())?)?;
    drop(listener);
    let state = temp.path().join("state");
    for mode in ["check", "diff", "apply", "check", "diff", "apply"] {
        let preview = matches!(mode, "check" | "diff");
        let before = snapshot(&payload)?;
        let events = process::run(cli, agent, &workload, &state, mode, Duration::from_secs(25))?;
        for name in TASKS {
            result(&events, name)?;
        }
        if preview {
            for name in [
                "Start disposable payload server",
                "Wait for disposable payload server",
                "Stop disposable payload server",
            ] {
                ensure!(
                    result(&events, name)?["skipped"] == true,
                    "{mode}: {name} did not skip"
                );
            }
            ensure!(
                snapshot(&payload)? == before,
                "{mode}: HTTP fixture mutated files"
            );
            ensure!(
                result(&events, "Verify payload server stopped")?["skipped"] == true,
                "{mode}: stop probe did not skip"
            );
        } else {
            for name in [
                "Start disposable payload server",
                "Wait for disposable payload server",
                "Stop disposable payload server",
                "Verify payload server stopped",
            ] {
                let task = result(&events, name)?;
                ensure!(
                    task["skipped"] != true && task["rc"] == 0,
                    "apply: {name} did not run successfully"
                );
            }
            ensure!(
                fs::read(payload.join("system/payload.txt"))? == b"synthetic-download",
                "apply: downloaded bytes mismatch"
            );
        }
        TcpListener::bind(address).context("HTTP fixture listener survived phase")?;
    }
    println!("PASS system-surface HTTP subset: apply, previews and cleanup");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::safe_root;
    use std::path::Path;

    #[test]
    fn rejects_shell_metacharacters_in_fixture_path() {
        assert!(safe_root(Path::new("/tmp/safe-root_123.4")).is_ok());
        for path in [
            "/tmp/a b",
            "/tmp/a;id",
            "/tmp/$(id)",
            "/tmp/`id`",
            "/tmp/a'",
            "/tmp/á",
        ] {
            assert!(safe_root(Path::new(path)).is_err(), "{path}");
        }
    }
}
