//! The PostgreSQL catalogue in `tests/workflow`.
//!
//! A role password is a secret alone, and every task that carries one is
//! never logged.
//!
//! The reload, leak and workload assertions live in
//! `workflow_postgres_nova.rs`, which the size policy keeps apart from
//! this file.

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
