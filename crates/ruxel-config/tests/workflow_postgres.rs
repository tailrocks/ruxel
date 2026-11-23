//! The PostgreSQL catalogue in `tests/workflow` and the nova workload in
//! `workflows/`.
//!
//! A role password is a secret alone, and every task that carries one is
//! never logged. The leak test walks every lowered task — plays, blocks and
//! handlers alike — and refuses a lookup outside a no-log task, so a secret
//! reference never reaches a plan line, a diff, or the ledger beside its
//! censored task.

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

/// A role password lowers to a lookup the engine resolves at apply time,
/// and the task that carries it is never logged.
#[test]
fn role_password_lowers_to_a_lookup_and_is_never_logged() -> Result {
    let config = valid("postgres-role.pkl")?;
    let task = tasks(&config)
        .into_iter()
        .find(|task| task["name"] == "Manage role chainargos")
        .ok_or("no chainargos task")?;
    assert_eq!(task["noLog"], serde_json::json!(true));
    assert_eq!(task["operation"]["kind"], "postgres_role");
    assert_eq!(task["operation"]["name"], "chainargos");
    assert_eq!(
        task["operation"]["password"],
        serde_json::json!({
            "kind": "lookup",
            "provider": "fnox",
            "key": "CHAINARGOS_PASSWORD",
            "argv": [],
        })
    );
    assert_eq!(task["operation"]["role_attr_flags"], "SUPERUSER");
    assert_eq!(task["operation"]["login_user"], "postgres");
    assert_eq!(task["operation"]["login_port"], 40000);
    assert!(task["operation"].get("state").is_none(), "{}", task);
    Ok(())
}

/// A secret role name lowers to a lookup too, and the plan line names the
/// step without its reference: the reference is never plan-visible, and the
/// task is never logged.
#[test]
fn secret_role_names_lower_to_lookups_and_stay_out_of_plan_lines() -> Result {
    let config = valid("postgres-role-secret-name.pkl")?;
    let task = tasks(&config)
        .into_iter()
        .find(|task| task["name"] == "Manage role with a secret name")
        .ok_or("no secret-name task")?;
    assert_eq!(task["noLog"], serde_json::json!(true));
    assert_eq!(
        task["operation"]["name"],
        serde_json::json!({
            "kind": "lookup",
            "provider": "op",
            "vault": "ChainArgos",
            "item": "ChainArgos bitcoin@postgresql-nova PostgreSQL",
            "field": "username",
            "argv": [],
        })
    );
    assert_eq!(
        task["operation"]["password"],
        serde_json::json!({
            "kind": "lookup",
            "provider": "fnox",
            "key": "PROCESSOR_VERIFY_BITCOIN_PASSWORD",
            "argv": [],
        })
    );
    Ok(())
}

/// Databases and schemas lower over the engine operations, and an owner may
/// be the secret the store holds.
#[test]
fn databases_and_schemas_lower_with_their_owners() -> Result {
    let config = valid("postgres-database-schema.pkl")?;
    let all = tasks(&config);
    let database = |dest: &str| {
        all.iter()
            .find(|task| task["operation"]["name"] == dest)
            .ok_or_else(|| format!("no {dest} task"))
    };
    let redshift = database("redshift")?;
    assert_eq!(redshift["operation"]["kind"], "postgres_database");
    assert_eq!(redshift["operation"]["owner"], "chainargos");
    assert_eq!(redshift["noLog"], serde_json::json!(false));
    let bitcoin = database("bitcoin")?;
    assert_eq!(bitcoin["noLog"], serde_json::json!(true));
    assert_eq!(
        bitcoin["operation"]["owner"],
        serde_json::json!({
            "kind": "lookup",
            "provider": "op",
            "vault": "ChainArgos",
            "item": "ChainArgos bitcoin@postgresql-nova PostgreSQL",
            "field": "username",
            "argv": [],
        })
    );
    let schema = all
        .iter()
        .find(|task| task["operation"]["kind"] == "postgres_schema")
        .ok_or("no schema task")?;
    assert_eq!(schema["operation"]["name"], "wallet_tags");
    assert_eq!(schema["operation"]["login_db"], "redshift");
    assert_eq!(schema["noLog"], serde_json::json!(false));
    Ok(())
}

/// Grants of every kind lower, and a database grant that names no objects
/// grants on the database itself. Privilege spelling is the author's own;
/// nothing here is a secret, so every grant stays visible.
#[test]
fn grants_lower_and_database_grants_default_their_objects() -> Result {
    let config = valid("postgres-privileges.pkl")?;
    let all = tasks(&config);
    assert_eq!(all.len(), 6);
    let grant = |kind: &str| {
        all.iter()
            .find(|task| task["operation"]["type"] == kind)
            .ok_or_else(|| format!("no {kind} grant"))
    };
    let database = grant("database")?;
    assert_eq!(database["operation"]["privs"], "CONNECT");
    assert_eq!(
        database["operation"]["objs"],
        serde_json::json!(["redshift"])
    );
    assert_eq!(database["operation"]["role"], "looker");
    let schema = grant("schema")?;
    assert_eq!(
        schema["operation"]["objs"],
        serde_json::json!(["public", "wallet_tags"])
    );
    let table = all
        .iter()
        .find(|task| {
            task["operation"]["type"] == "table" && task["operation"]["schema"] == "wallet_tags"
        })
        .ok_or("no wallet_tags table grant")?;
    assert_eq!(
        table["operation"]["objs"],
        serde_json::json!(["ALL_IN_SCHEMA"])
    );
    let defaults: Vec<&&serde_json::Value> = all
        .iter()
        .filter(|task| task["operation"]["type"] == "default_privs")
        .collect();
    assert_eq!(defaults.len(), 2);
    for default in &defaults {
        assert!(default["operation"].get("target_roles").is_none());
    }
    let default = defaults
        .iter()
        .find(|task| task["operation"]["schema"] == "wallet_tags")
        .ok_or("no wallet_tags default grant")?;
    assert_eq!(default["operation"]["schema"], "wallet_tags");
    for task in &all {
        assert_eq!(task["noLog"], serde_json::json!(false), "{}", task["name"]);
    }
    Ok(())
}

/// A table grant over `ALL_IN_SCHEMA` names its schema: without one the
/// grant names no tables.
#[test]
fn a_table_grant_over_all_in_schema_names_its_schema() -> Result {
    let path = root()
        .join("invalid")
        .join("postgres-privilege-table-all-in-schema-no-schema.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "a table grant over ALL_IN_SCHEMA names its schema; \
                    without one the grant names no tables";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// A blank privilege entry names nothing: the listing element that is
/// whitespace alone is refused instead of silently dropped.
#[test]
fn a_grant_refuses_a_blank_privilege_entry() -> Result {
    let path = root()
        .join("invalid")
        .join("postgres-privilege-blank-priv-entry.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "a blank privilege names nothing; \
                    drop the blank entry or name the privilege";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// The schema rides table and default-privilege grants alone: a database
/// grant names none.
#[test]
fn a_database_grant_names_no_schema() -> Result {
    let path = root()
        .join("invalid")
        .join("postgres-privilege-schema-on-database.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "the schema rides table and default-privilege grants alone; \
                    a database grant names no schema";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// Default privileges name their schema, not objects: the future tables of
/// the schema carry them, so the grant names no `objs`.
#[test]
fn default_privileges_name_no_objects() -> Result {
    let path = root()
        .join("invalid")
        .join("postgres-privilege-default-privs-objs.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "default privileges name their schema, not objects; \
                    a default_privs grant names no objects";
    assert!(error.contains(expected), "{error}");
    Ok(())
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
