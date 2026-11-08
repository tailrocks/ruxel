use crate::{docker_fixture::Docker, host_fixture::Guest, pins};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{fs, io::Read, os::unix::fs::OpenOptionsExt, path::Path};

const ASSETS: [(&str, &str); 2] = [
    (
        "source.txt",
        "b2c70bba7bf0ef43959341e03b923efd23fed1a0b3ed1895736736d88a5b39b0",
    ),
    (
        "template.pkl",
        "e848f3f641a2b66a80d18c9af88a4906d313545d206b43f1d1d3a9eeb5ee1eab",
    ),
];
const SHIM: &str = r#"#!/bin/sh
case "$1" in
  list-units) printf '%s\n' 'ssh.service loaded active running synthetic' ;;
  list-unit-files) printf '%s\n' 'ssh.service enabled' ;;
  restart) [ "$2" = ssh ] || exit 1; printf 'restart\n' >> /tmp/work/payload/handler.log ;;
  *) exit 1 ;;
esac
"#;
const VERIFY_FILE_DETAILS: &str = r#"set -eu
cd "$1"
test ! -e absent && test ! -L absent
awk 'previous == "alpha" && $0 == "beta" { found = 1 } { previous = $0 } END { exit !found }' block.txt
test -f created-by-command
printf synthetic | cmp -s - created-by-shell
"#;

pub(super) fn stage(root: &Path, stage: &Path, docker: &Docker) -> Result<()> {
    docker.exec(
        &[
            "/bin/bash",
            "-euc",
            "mkdir -p /tmp/work/assets; : > /tmp/work/payload/handler.log",
        ],
        0,
    )?;
    let shim = stage.join("systemctl");
    fs::write(&shim, SHIM)?;
    docker.copy(&shim, "/opt/ruxel-fixture/systemctl")?;
    docker.exec(&["/bin/chmod", "755", "/opt/ruxel-fixture/systemctl"], 0)?;
    for (name, hash) in ASSETS {
        let source = root.join("testdata/workloads/pr66/assets").join(name);
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits())?)
            .open(&source)?;
        ensure!(
            file.metadata()?.is_file(),
            "fixture asset is not a regular file"
        );
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 65536, "fixture asset too large");
        ensure!(
            pins::digest(&bytes) == hash,
            "fixture asset pin mismatch: {name}"
        );
        let bytes = if name == "template.pkl" {
            std::str::from_utf8(&bytes)?
                .replace("../../../../schema/Ruxel.pkl", "../schema/Ruxel.pkl")
                .into_bytes()
        } else {
            bytes
        };
        let file = stage.join(name);
        fs::write(&file, bytes)?;
        docker.copy(&file, &format!("/tmp/work/assets/{name}"))?;
    }
    Ok(())
}

fn task<'a>(events: &'a [Value], name: &str) -> Result<&'a Value> {
    let mut matches = events
        .iter()
        .filter(|event| event["host"] == "fixture" && event["task"] == name);
    let found = matches.next().context("missing fixture task")?;
    ensure!(
        matches.next().is_none() && found["result"]["failed"] == false,
        "invalid fixture task {name}"
    );
    Ok(&found["result"])
}

fn verify_events(events: &[Value], handler: bool) -> Result<()> {
    for name in [
        "Create synthetic fixture root",
        "Exercise recursive directory attributes",
        "Exercise directory path",
        "Exercise symbolic link",
        "Exercise absent path",
        "Copy literal content",
        "Copy synthetic source asset",
        "Render synthetic template",
        "Exercise line replacement",
        "Exercise multiline replacement",
        "Exercise managed block",
        "Inspect path with follow",
        "Inspect templated registered path",
        "Read synthetic content",
        "Preserve decoded fact shape",
        "Report mixed template shape",
        "Exercise command mapping and creates",
        "Exercise command scalar",
        "Exercise shell arguments",
        "Exercise assertion message shape",
        "Exercise guarded failure module",
    ] {
        task(events, name)?;
    }
    let handlers: Vec<_> = events
        .iter()
        .filter(|e| e["host"] == "fixture" && e["task"] == "Restart synthetic fixture service")
        .collect();
    ensure!(
        handlers.len() == usize::from(handler),
        "handler event count mismatch"
    );
    if handler {
        ensure!(
            handlers[0]["result"]["changed"] == true && handlers[0]["result"]["failed"] == false,
            "handler result mismatch"
        );
    }
    Ok(())
}

fn observe(guest: &Guest, events: &[Value], expected_handlers: usize) -> Result<()> {
    let shell = r#"set -eu
cd /tmp/work/payload/files
test -d tree && test "$(stat -c '%a:%u:%g' tree)" = 700:0:0
test -d dest-dir && test -L tree-link && test "$(readlink tree-link)" = /tmp/work/payload/files/tree
test "$(stat -c '%a:%u:%g' copied.txt)" = 644:0:0
printf 'replaced\n' | cmp -s - copied.txt
printf 'synthetic source\n' | cmp -s - source.txt
test "$(stat -c '%a:%u:%g' rendered.txt)" = 640:0:0
printf 'alpha\nbeta\njoined=alpha,beta\nhash=b3cc0475bb78a5026098858e9889acf666d31062d513d303314eca31d36e72f2\noptional=fallback\nencoded=synthetic%%20value\nconditional=enabled\n' | cmp -s - rendered.txt
"#;
    guest.docker.exec(&["/bin/bash", "-euc", shell], 0)?;
    guest.docker.exec(
        &[
            "/bin/bash",
            "-euc",
            VERIFY_FILE_DETAILS,
            "--",
            "/tmp/work/payload/files",
        ],
        0,
    )?;
    ensure!(
        task(events, "Inspect path with follow")?["stat"]["exists"] == true,
        "stat mismatch"
    );
    ensure!(
        task(events, "Read synthetic content")?["content"] == "cmVwbGFjZWQK",
        "slurp mismatch"
    );
    ensure!(
        task(events, "Preserve decoded fact shape")?["facts"]["fixture_decoded"] == "replaced",
        "decoded fact mismatch"
    );
    ensure!(
        task(events, "Report mixed template shape")?["msg"] == "decoded=replaced count=2",
        "debug result mismatch"
    );
    ensure!(
        task(events, "Exercise assertion message shape")?["failed"] == false,
        "assertion failed"
    );
    ensure!(
        task(events, "Exercise guarded failure module")?["skipped"] == true,
        "guarded task not skipped"
    );
    let handler_log = guest.text(&["/bin/cat", "/tmp/work/payload/handler.log"])?;
    ensure!(
        handler_log.lines().count() == expected_handlers
            && handler_log.lines().all(|s| s == "restart"),
        "handler count mismatch"
    );
    Ok(())
}

pub(super) fn execute(guest: &Guest) -> Result<()> {
    for phase in ["fresh", "converged", "check", "diff", "drift_repair"] {
        if phase == "check" {
            guest.docker.exec(&["/bin/bash", "-euc", "printf drift > /tmp/work/payload/files/copied.txt; rm /tmp/work/payload/files/source.txt"], 0)?;
        }
        let preview = matches!(phase, "check" | "diff");
        let before = if preview {
            Some((guest.snapshot()?, fixture_inputs(guest)?))
        } else {
            None
        };
        let events = guest.run(if preview { phase } else { "apply" }, 0)?;
        verify_events(&events, phase == "fresh")?;
        if let Some((before, inputs)) = before {
            ensure!(
                guest.snapshot()? == before && fixture_inputs(guest)? == inputs,
                "{phase} mutated fixture state"
            );
        }
        if preview {
            ensure!(
                task(&events, "Copy literal content")?["changed"] == true,
                "{phase}: missing content drift prediction"
            );
            ensure!(
                task(&events, "Copy synthetic source asset")?["changed"] == true,
                "{phase}: missing asset drift prediction"
            );
            ensure!(
                task(&events, "Read synthetic content")?["content"] == "ZHJpZnQ=",
                "{phase}: expected unchanged drift"
            );
            if phase == "diff" {
                ensure!(
                    events.iter().any(|event| event["result"]["diff"]
                        .as_str()
                        .is_some_and(|diff| !diff.is_empty())),
                    "diff: missing change details"
                );
            }
        } else {
            observe(guest, &events, 1)?;
        }
        println!("PASS files-content {phase}");
    }
    Ok(())
}

fn fixture_inputs(guest: &Guest) -> Result<Vec<u8>> {
    guest.docker.exec(&["/bin/bash", "-euc", "stat -c '%n %f %u %g %s %y %z' /tmp/work/assets /tmp/work/assets/source.txt /tmp/work/assets/template.pkl /opt/ruxel-fixture/systemctl; sha256sum /tmp/work/assets/source.txt /tmp/work/assets/template.pkl /opt/ruxel-fixture/systemctl"], 0)
}

#[cfg(test)]
mod tests {
    use super::{VERIFY_FILE_DETAILS, task};
    use serde_json::json;
    use std::{fs, os::unix::fs::symlink, process::Command};

    #[test]
    fn observations_require_unique_successful_fixture_tasks() {
        let good = json!({"host":"fixture", "task":"example", "result":{"failed":false}});
        assert!(task(std::slice::from_ref(&good), "example").is_ok());
        assert!(task(&[], "example").is_err());
        assert!(task(&[good.clone(), good], "example").is_err());
        assert!(
            task(
                &[json!({"host":"fixture", "task":"example", "result":{"failed":true}})],
                "example"
            )
            .is_err()
        );
    }

    #[test]
    fn file_observations_reject_broken_link_extra_bytes_and_separated_block() -> anyhow::Result<()>
    {
        let temp = tempfile::tempdir()?;
        let path = temp.path();
        fs::write(path.join("created-by-command"), "")?;
        fs::write(path.join("created-by-shell"), "synthetic")?;
        fs::write(path.join("block.txt"), "prefix\nalpha\nbeta\nsuffix\n")?;
        let valid = || -> anyhow::Result<bool> {
            Ok(Command::new("/bin/bash")
                .args(["-euc", VERIFY_FILE_DETAILS, "--"])
                .arg(path)
                .status()?
                .success())
        };
        assert!(valid()?);
        symlink("missing", path.join("absent"))?;
        assert!(!valid()?);
        fs::remove_file(path.join("absent"))?;
        fs::write(path.join("created-by-shell"), "synthetic\n")?;
        assert!(!valid()?);
        fs::write(path.join("created-by-shell"), "synthetic")?;
        fs::write(path.join("block.txt"), "beta\nother\nalpha\n")?;
        assert!(!valid()?);
        fs::write(path.join("block.txt"), "alpha\nother\nbeta\n")?;
        assert!(!valid()?);
        Ok(())
    }
}
