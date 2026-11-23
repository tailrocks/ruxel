//! Track A's Lowering on the corpus: a fetched script lowers to a download
//! converging the destination and a guarded run, line entries lower to the
//! engine's lineinfile and replace operations, and a notice lowers to the
//! engine's debug operation that never reports a change.

use std::path::{Path, PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn root() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/workflow")).to_path_buf()
}

fn options() -> ruxel_config::EvalOptions {
    ruxel_config::EvalOptions {
        root_dir: Some(root()),
        ..ruxel_config::EvalOptions::default()
    }
}

fn valid(name: &str) -> Result<serde_json::Value> {
    Ok(ruxel_config::eval_config(
        &root().join("valid").join(name),
        &options(),
    )?)
}

fn tasks(config: &serde_json::Value) -> Vec<&serde_json::Value> {
    config["plays"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|play| play["tasks"].as_array().into_iter().flatten())
        .collect()
}

/// A fetched script lowers to a download converging the destination and a
/// guarded run under `/bin/bash` (G5): the download registers beside the
/// run, and the watcher's guard reads both results, so a changed download
/// with a skipped run still reports the step's change.
#[test]
fn fetched_script_downloads_then_runs_guarded() -> Result {
    let config = valid("fetched-script.pkl")?;
    let all = tasks(&config);
    let position = |name: &str| {
        all.iter()
            .position(|task| task["name"] == name)
            .ok_or_else(|| format!("no task {name:?}"))
    };
    let download = position("Download /opt/holla-install.sh")?;
    let probe = position("Check whether /opt/holla/.installed exists")?;
    let run = position("Fetch and run the holla installer")?;
    let watcher = position("Show the installed version")?;
    assert_eq!(probe, download + 1, "the download converges first");
    assert_eq!(run, probe + 1, "then the guarded run");
    assert_eq!(
        all[download]["operation"],
        serde_json::json!({"kind": "get_url", "dest": "/opt/holla-install.sh", "url": "https://get.example.test/holla/install.sh"}),
        "the download converges the destination: {download:#?}",
    );
    assert_eq!(
        all[run]["operation"]["argv"],
        serde_json::json!([
            "/bin/bash",
            "/opt/holla-install.sh",
            "--prefix",
            "/opt/holla",
        ]),
        "the run executes the download: {run:#?}",
    );
    assert_eq!(
        all[run]["operation"]["creates"], "/opt/holla/.installed",
        "the run carries its guard: {run:#?}",
    );
    let register = all[run]["register"]
        .as_str()
        .ok_or("the run registers its result")?;
    assert_eq!(
        all[download]["register"].as_str(),
        Some(format!("{register}_download").as_str()),
        "the download registers beside the run",
    );
    let condition = all[watcher]["when"].to_string();
    for result in [register, &format!("{register}_download")] {
        assert!(
            condition.contains(&format!("\"{result}\"")),
            "{result}: {condition}",
        );
    }
    Ok(())
}

/// Line entries lower to the engine's lineinfile and replace operations
/// with the effective match: the named match, or the line itself.
#[test]
fn line_entries_lower_to_their_operations() -> Result {
    let config = valid("ensure-line.pkl")?;
    let lines: Vec<&serde_json::Value> = tasks(&config)
        .into_iter()
        .filter(|task| {
            ["lineinfile", "replace"]
                .contains(&task["operation"]["kind"].as_str().unwrap_or_default())
        })
        .collect();
    assert_eq!(lines.len(), 3, "two entries and a replacement: {lines:#?}");
    let port = lines
        .iter()
        .find(|task| task["name"] == "Set the holla port")
        .ok_or("no port entry")?;
    assert_eq!(
        port["operation"],
        serde_json::json!({"kind": "lineinfile", "path": "/etc/holla/holla.conf", "line": "port = 8080", "regexp": "port = 8080", "state": "present"}),
    );
    let host = lines
        .iter()
        .find(|task| task["name"] == "Set the holla host")
        .ok_or("no host entry")?;
    assert_eq!(host["operation"]["line"], "host = 0.0.0.0");
    assert_eq!(host["operation"]["regexp"], "host\\s*=.*");
    let debug = lines
        .iter()
        .find(|task| task["name"] == "Drop the debug flag")
        .ok_or("no debug replacement")?;
    assert_eq!(
        debug["operation"],
        serde_json::json!({"kind": "replace", "path": "/etc/holla/holla.conf", "regexp": "^debug = true$", "replace": "debug = false"}),
    );
    Ok(())
}

/// A notice lowers to the engine's debug operation and never reports a
/// change: a static one always prints, and a capture-backed one prints
/// only when the report is not empty.
#[test]
fn notices_print_without_reporting_change() -> Result {
    let config = valid("notice.pkl")?;
    let notices: Vec<&serde_json::Value> = tasks(&config)
        .into_iter()
        .filter(|task| task["operation"]["kind"] == "debug")
        .collect();
    assert_eq!(notices.len(), 2, "two notices: {notices:#?}");
    let report = notices
        .iter()
        .find(|task| task["name"] == "Report the terminal")
        .ok_or("no report notice")?;
    assert!(
        report["operation"]["msg"]
            .to_string()
            .contains("Controller terminfo:"),
        "the notice renders the report: {report:#}",
    );
    assert!(
        report["when"].to_string().contains("\"stdout\""),
        "the notice prints only when the report is not empty: {report:#}",
    );
    let ready = notices
        .iter()
        .find(|task| task["name"] == "Holla is ready")
        .ok_or("no ready notice")?;
    assert_eq!(ready["operation"]["msg"], "Holla is ready");
    assert!(
        ready.get("when").is_none_or(serde_json::Value::is_null),
        "a static notice always prints: {ready:#}"
    );
    let never = serde_json::json!({"kind": "literal", "value": false});
    for notice in notices {
        assert_eq!(
            notice["changedWhen"], never,
            "a notice never reports a change: {notice:#}",
        );
    }
    Ok(())
}
