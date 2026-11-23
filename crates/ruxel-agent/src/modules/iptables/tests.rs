#[path = "tests/absent.rs"]
mod absent;
#[path = "tests/alias.rs"]
mod alias;
#[path = "tests/duplicates.rs"]
mod duplicates;
#[path = "tests/parsing.rs"]
mod parsing;
#[path = "tests/position.rs"]
mod position;
#[path = "tests/present.rs"]
mod present;
#[path = "tests/probe.rs"]
mod probe;
use std::os::unix::fs::PermissionsExt;

use super::{ExecContext, run_with};

fn context(check_mode: bool) -> ExecContext {
    ExecContext {
        check_mode,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    }
}

/// A stub that stands in for iptables: the rule is there when the marker
/// file is, and every call is written to the log.
fn stub(dir: &std::path::Path, present: bool) -> (String, std::path::PathBuf, std::path::PathBuf) {
    let marker = dir.join("rule-present");
    if present {
        std::fs::write(&marker, "rule\n").unwrap();
    } else {
        let _ = std::fs::remove_file(&marker);
    }
    let log = dir.join("calls.log");
    let script = dir.join("iptables-stub");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\ncase \"$1\" in\n  -S) if [ -f '{marker}' ]; then echo \"-P $2 ACCEPT\"; echo \"-A $2 -p tcp -j DROP\"; fi; exit 0;;\n  -C) if [ -f '{marker}' ]; then exit 0; else echo 'iptables: Bad rule (does a matching rule exist in that chain?).' >&2; exit 1; fi;;\n  -D) rm -f '{marker}'; exit 0;;\n  -A) touch '{marker}'; exit 0;;\nesac\nexit 0\n",
            log = log.display(),
            marker = marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    (script.to_str().unwrap().to_owned(), marker, log)
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ruxel-iptables-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
