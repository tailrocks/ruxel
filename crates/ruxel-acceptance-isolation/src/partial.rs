use crate::common::{Harness, events, pkl, recaps, task};
use anyhow::{Result, ensure};
use std::{fs, path::Path};
const SENTINEL: &str = "synthetic-sensitive-missing-reference-835792";
pub fn check(cli: &Path, agent: &Path) -> Result<()> {
    let h = Harness::new(cli, agent)?;
    for private in [false, true] {
        let name = if private { "private" } else { "ordinary" };
        let payload = h.path(name);
        fs::create_dir(&payload)?;
        let source = include_str!("../fixtures/partial.pkl")
            .replace("PREFIX", &pkl(&format!("{}/", payload.display()))?)
            .replace("PRIVATE", if private { "true" } else { "false" })
            .replace(
                "SECRET",
                &pkl(if private {
                    SENTINEL
                } else {
                    "ordinary_missing"
                })?,
            );
        h.write(&source)?;
        let log = h.path(&format!("{name}-events"));
        let output = h.run(agent, &format!("{name}-state"), "1", Some(&log))?;
        ensure!(log.metadata()?.len() > 0, "missing event log");
        let logged = fs::read_to_string(log)?;
        ensure!(
            ![&output.stdout, &output.stderr, &logged]
                .iter()
                .any(|s| s.contains(SENTINEL)),
            "noLog failure leaked synthetic value"
        );
        let e = events(&output, 8)?;
        for name in ["fixture-a", "fixture-b"] {
            ensure!(
                fs::read_to_string(payload.join(name))? == "synthetic",
                "initial copy missing"
            );
        }
        let writes = task(&e, "write");
        ensure!(
            writes.len() == 2
                && writes.iter().any(|r| r["host"] == "fixture-a")
                && writes.iter().any(|r| r["host"] == "fixture-b")
                && writes.iter().all(|r| r["result"]["changed"] == true),
            "missing successful initial writes"
        );
        let (bad, good) = recaps(&e)?;
        for field in ["ok", "changed", "failed"] {
            ensure!(
                bad[field].as_u64().is_some_and(|n| n >= 1),
                "failed host lost {field}"
            );
        }
        ensure!(
            good["failed"] == 0 && good["changed"].as_u64().is_some_and(|n| n >= 1),
            "healthy recap incorrect"
        );
        let continued = task(&e, "sibling continues");
        ensure!(
            continued.len() == 1
                && continued[0]["host"] == "fixture-b"
                && continued[0]["result"]["msg"] == "continued",
            "healthy sibling did not continue"
        );
    }
    println!("PASS partial recaps: progress retained, sibling continues, noLog redacted");
    Ok(())
}
