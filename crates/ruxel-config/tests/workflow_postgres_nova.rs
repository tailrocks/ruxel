//! The PostgreSQL reload wiring and the nova workload: the assertions of
//! the corpus in `tests/workflow` that the access file, the restart
//! report, the secret-leak sweep and the nova workload need.
//!
//! The catalogue harness itself lives in `workflow_postgres.rs`; this file
//! holds the reload, leak and workload assertions, which the size policy
//! keeps apart from it.

use std::path::{Path, PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn root() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/workflow")).to_path_buf()
}

fn workflows() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../workflows")).to_path_buf()
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

fn workload() -> Result<serde_json::Value> {
    Ok(ruxel_config::eval_config(
        &workflows().join("setup-postgresql-nova.pkl"),
        &ruxel_config::EvalOptions {
            root_dir: Some(workflows()),
            ..ruxel_config::EvalOptions::default()
        },
    )?)
}

/// Every task of every play, including block children and handlers: the
/// units the leak test walks.
fn all_tasks(config: &serde_json::Value) -> Vec<&serde_json::Value> {
    fn collect<'value>(
        task: &'value serde_json::Value,
        found: &mut Vec<&'value serde_json::Value>,
    ) {
        found.push(task);
        for section in ["block", "rescue", "always"] {
            if let Some(children) = task.get(section).and_then(|tasks| tasks.as_array()) {
                for child in children {
                    collect(child, found);
                }
            }
        }
    }
    let mut found = Vec::new();
    for play in config["plays"].as_array().into_iter().flatten() {
        for section in ["preTasks", "tasks", "handlers"] {
            for task in play[section].as_array().into_iter().flatten() {
                collect(task, &mut found);
            }
        }
    }
    found
}

fn tasks(config: &serde_json::Value) -> Vec<&serde_json::Value> {
    config["plays"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|play| play["tasks"].as_array().into_iter().flatten())
        .collect()
}

/// The rendered access file is 0640 owned by postgres with secrets in its
/// body, so its task is never logged; the service reloads on it and never
/// restarts on it.
#[test]
fn access_file_renders_at_0640_and_reloads() -> Result {
    let config = valid("postgres-pg-hba.pkl")?;
    let all = tasks(&config);
    let file = all
        .iter()
        .find(|task| task["name"] == "Write /etc/postgresql/18/main/pg_hba.conf")
        .ok_or("no pg_hba task")?;
    assert_eq!(file["operation"]["kind"], "copy");
    assert_eq!(file["operation"]["mode"], "0640");
    assert_eq!(file["operation"]["owner"], "postgres");
    assert_eq!(file["operation"]["group"], "postgres");
    assert_eq!(file["noLog"], serde_json::json!(true));
    let content = file["operation"]["content"].to_string();
    assert!(content.contains("scram-sha-256"), "{content}");
    assert!(
        content.contains("lookup"),
        "usernames resolve at apply: {content}"
    );
    assert_eq!(
        file["notify"],
        serde_json::json!(["Reload postgresql@18-main"])
    );
    let handlers: Vec<&serde_json::Value> = config["plays"][0]["handlers"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let reload = handlers
        .iter()
        .find(|handler| handler["name"] == "Reload postgresql@18-main")
        .ok_or("no reload handler")?;
    assert_eq!(reload["operation"]["state"], "reloaded");
    assert!(
        handlers
            .iter()
            .all(|handler| handler["operation"]["state"] != "restarted"),
        "PostgreSQL is never restarted"
    );
    Ok(())
}

/// The restart report reads the settings a reload could not adopt and warns
/// about them. The capture runs in check and diff too, the notice fires
/// only on a non-empty report, and it never reports a change.
#[test]
fn restart_report_warns_without_restarting() -> Result {
    let config = valid("postgres-pending-restart.pkl")?;
    let all = tasks(&config);
    assert_eq!(all.len(), 2);
    let report = all
        .iter()
        .find(|task| task["name"] == "Report settings that still require a restart")
        .ok_or("no report capture")?;
    assert_eq!(report["checkMode"], serde_json::json!(false));
    assert!(
        report["operation"]["argv"]
            .as_array()
            .is_some_and(|argv| argv.iter().any(|argument| argument
                .as_str()
                .is_some_and(|text| text.contains("pending_restart")))),
        "{}",
        report["operation"]
    );
    let notice = all
        .iter()
        .find(|task| task["name"] == "Warn when a restart is needed to finish applying the config")
        .ok_or("no notice")?;
    assert_eq!(notice["operation"]["kind"], "debug");
    assert_eq!(notice["noLog"], serde_json::json!(false));
    assert_eq!(
        notice["changedWhen"],
        serde_json::json!({"kind": "literal", "value": false})
    );
    let message = notice["operation"]["msg"].to_string();
    assert!(
        message.contains("pg_ctlcluster 18 main restart"),
        "{message}"
    );
    assert_eq!(notice["when"]["function"], "ne", "{}", notice["when"]);
    assert!(
        notice["when"].to_string().contains("results"),
        "{}",
        notice["when"]
    );
    assert!(
        config["plays"][0]["handlers"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|handler| handler["operation"]["state"] != "restarted"),
        "PostgreSQL is never restarted"
    );
    Ok(())
}

/// No secret reference escapes a no-log task: every lookup in the lowered
/// configuration sits inside a task the plan censors, and no plan-visible
/// name carries one. The whole lowered file is the assertion's input, so a
/// new secret-bearing step that forgets its no-log fails here.
#[test]
fn no_lookup_escapes_a_no_log_task() -> Result {
    for name in [
        "postgres-role.pkl",
        "postgres-role-secret-name.pkl",
        "postgres-database-schema.pkl",
        "postgres-privileges.pkl",
        "postgres-pg-hba.pkl",
        "postgres-pending-restart.pkl",
    ] {
        leaks_nowhere(&valid(name)?, name);
    }
    leaks_nowhere(&workload()?, "setup-postgresql-nova.pkl");
    Ok(())
}

fn leaks_nowhere(config: &serde_json::Value, name: &str) {
    for task in all_tasks(config) {
        if contains_lookup(task) {
            assert_eq!(
                task["noLog"],
                serde_json::json!(true),
                "{name}: a task that carries a secret is logged: {}",
                task["name"]
            );
        }
        for visible in [task["name"].as_str(), task["register"].as_str()] {
            assert!(
                visible.is_none_or(|text| !text.contains("__ruxel_secret_8d41.")),
                "{name}: a secret reference reaches a plan-visible field: {visible:?}"
            );
        }
    }
}

/// Whether the value carries a secret reference: a lookup the engine
/// resolves at apply time. The lowered configuration never holds a secret
/// value — evaluation contacts no store — so the lookup is what must stay
/// inside a task the plan censors.
fn contains_lookup(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(fields) => {
            fields.get("kind").and_then(|kind| kind.as_str()) == Some("lookup")
                || fields.values().any(contains_lookup)
        }
        serde_json::Value::Array(items) => items.iter().any(contains_lookup),
        _ => false,
    }
}

/// The nova workload lowers end to end: the superuser role, the access
/// file at 0640, the reload wiring, and the restart report all survive.
#[test]
fn nova_workload_lowers_with_its_roles_and_reload_wiring() -> Result {
    let config = workload()?;
    assert!(!config["plays"].as_array().is_none_or(Vec::is_empty));
    let all = tasks(&config);
    let chainargos = all
        .iter()
        .find(|task| {
            task["operation"]["kind"] == "postgres_role"
                && task["operation"]["name"] == "chainargos"
        })
        .ok_or("no chainargos role")?;
    assert_eq!(chainargos["noLog"], serde_json::json!(true));
    assert_eq!(
        chainargos["operation"]["password"],
        serde_json::json!({
            "kind": "lookup",
            "provider": "op",
            "vault": "ChainArgos",
            "item": "ChainArgos chainargos@titan PostgreSQL",
            "field": "password",
            "argv": [],
        })
    );
    assert_eq!(chainargos["operation"]["role_attr_flags"], "SUPERUSER");
    let roles = all
        .iter()
        .filter(|task| task["operation"]["kind"] == "postgres_role")
        .count();
    assert_eq!(roles, 7, "chainargos, four chain roles, looker, kestra");
    let databases = all
        .iter()
        .filter(|task| task["operation"]["kind"] == "postgres_database")
        .count();
    assert_eq!(databases, 18);
    let file = all
        .iter()
        .find(|task| task["operation"]["dest"] == "/etc/postgresql/18/main/pg_hba.conf")
        .ok_or("no pg_hba file")?;
    assert_eq!(file["operation"]["mode"], "0640");
    assert_eq!(file["noLog"], serde_json::json!(true));
    for dest in [
        "/etc/postgresql/18/main/postgresql.conf",
        "/etc/postgresql/18/main/pg_hba.conf",
    ] {
        let watched = all
            .iter()
            .find(|task| task["operation"]["dest"] == dest)
            .ok_or_else(|| format!("no {dest} file"))?;
        assert_eq!(
            watched["notify"],
            serde_json::json!(["Reload postgresql@18-main"]),
            "{dest}"
        );
    }
    assert!(
        all.iter()
            .any(|task| task["name"] == "Report settings that still require a restart")
    );
    assert!(
        all.iter()
            .any(|task| task["name"]
                == "Warn when a restart is needed to finish applying the config")
    );
    Ok(())
}
