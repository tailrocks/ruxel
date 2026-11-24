//! The PostgreSQL catalogue guards in `tests/workflow`: connection
//! propagation, refused states, and order-free grant identity.
//!
//! The catalogue harness itself lives in `workflow_postgres.rs`, which the
//! size policy keeps apart from these guards.

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

/// A grant carries the connection it manages: every grant of the corpus
/// states its login user and port, and the engine receives them instead of
/// falling back to the agent defaults.
#[test]
fn grants_carry_their_login_user_port_and_become_user() -> Result {
    let config = valid("postgres-privileges.pkl")?;
    let all = tasks(&config);
    assert_eq!(all.len(), 6);
    for task in &all {
        assert_eq!(
            task["operation"]["login_user"], "postgres",
            "{}",
            task["name"]
        );
        assert_eq!(task["operation"]["login_port"], 40000, "{}", task["name"]);
        assert_eq!(task["becomeUser"], "postgres", "{}", task["name"]);
    }
    Ok(())
}

/// A database grant names no objects: it grants on the database itself,
/// which loginDb already names.
#[test]
fn a_database_grant_names_no_objects() -> Result {
    let path = root()
        .join("invalid")
        .join("postgres-privilege-database-objs.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "a database grant names no objects; it grants on the database itself";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// `ALL_IN_SCHEMA` names every table of the schema, so it rides alone.
#[test]
fn all_in_schema_rides_alone() -> Result {
    let path = root()
        .join("invalid")
        .join("postgres-privilege-all-in-schema-mixed.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "ALL_IN_SCHEMA names every table of the schema, so it rides alone";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// ALL grants every privilege of the kind, so it rides alone beside no
/// other privilege.
#[test]
fn all_rides_alone_beside_no_other_privilege() -> Result {
    let path = root()
        .join("invalid")
        .join("postgres-privilege-all-mixed.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "ALL grants every table privilege, so it rides alone";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// Peer authentication checks the operating-system user over the
/// Unix-domain socket, so a TCP rule takes no peer method.
#[test]
fn a_host_rule_takes_no_peer_method() -> Result {
    let path = root().join("invalid").join("postgres-pg-hba-host-peer.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "pg_hba method \"peer\" authenticates the client's operating-system user";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// Certificate authentication checks the client's TLS certificate, so a
/// rule without encryption takes no cert method.
#[test]
fn a_host_rule_takes_no_cert_method() -> Result {
    let path = root().join("invalid").join("postgres-pg-hba-host-cert.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "pg_hba method \"cert\" checks the client's TLS certificate";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// Empty catalogue names fail before the run starts, never as zero-length
/// SQL identifiers during apply.
#[test]
fn empty_catalogue_names_are_refused() -> Result {
    for (file, expected) in [
        (
            "postgres-role-empty-name.pkl",
            "the role's name is empty; name the role",
        ),
        (
            "postgres-database-empty-name.pkl",
            "the database's name is empty; name the database",
        ),
        (
            "postgres-schema-empty-name.pkl",
            "the schema's name is empty; name the schema",
        ),
        (
            "postgres-schema-empty-login-db.pkl",
            "the schema's database is empty; name the database that holds the schema",
        ),
        (
            "postgres-privilege-empty-login-db.pkl",
            "the grant's database is empty; name the database the grant lives in",
        ),
        (
            "postgres-privilege-empty-schema.pkl",
            "the grant's schema is empty; a table grant names its schema",
        ),
        (
            "postgres-privilege-empty-objs.pkl",
            "a granted object is empty; name every object the grant is on",
        ),
        (
            "postgres-privilege-empty-target-role.pkl",
            "a target role is empty; name the roles whose future tables carry the grant",
        ),
    ] {
        let path = root().join("invalid").join(file);
        let error = ruxel_config::eval_config(&path, &options())
            .err()
            .ok_or_else(|| format!("{file} evaluated"))?
            .to_string();
        assert!(error.contains(expected), "{file}: {error}");
    }
    Ok(())
}

/// A file built beside the renderer carries no unchecked line: the
/// lowering checks the carried rules with the renderer's own sentences.
#[test]
fn a_directly_built_access_file_checks_its_rules() -> Result {
    let path = root()
        .join("invalid")
        .join("postgres-pg-hba-direct-bad-rule.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "a local pg_hba rule matches Unix-domain sockets, so it takes no address";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// Roles and databases carry the database they connect to: an
/// administrative login role seldom names a database of its own, so without
/// one the operation would fail before its first query.
#[test]
fn roles_and_databases_carry_their_login_database() -> Result {
    let role_config = valid("postgres-role.pkl")?;
    let role = tasks(&role_config)
        .into_iter()
        .find(|task| task["operation"]["kind"] == "postgres_role")
        .ok_or("no role task")?;
    assert_eq!(role["operation"]["login_db"], "postgres");
    let database_config = valid("postgres-database-schema.pkl")?;
    let database = tasks(&database_config)
        .into_iter()
        .find(|task| task["operation"]["name"] == "redshift")
        .ok_or("no database task")?;
    assert_eq!(database["operation"]["login_db"], "postgres");
    Ok(())
}

/// The login database is the database the agent connects to, not a blank.
#[test]
fn an_empty_login_database_is_refused() -> Result {
    let path = root()
        .join("invalid")
        .join("postgres-role-empty-login-db.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "the login database is empty; name the database the agent connects to";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// Names past PostgreSQL's 63-byte identifier limit fail before the run
/// starts, instead of creating a silently truncated object the next apply
/// mistakes for absent.
#[test]
fn overlong_names_are_refused() -> Result {
    for (file, expected) in [
        (
            "postgres-role-long-name.pkl",
            "the role's name is 64 characters; PostgreSQL truncates identifiers past 63 bytes",
        ),
        (
            "postgres-pg-hba-long-user.pkl",
            "(64 characters); PostgreSQL truncates identifiers past 63 bytes",
        ),
    ] {
        let path = root().join("invalid").join(file);
        let error = ruxel_config::eval_config(&path, &options())
            .err()
            .ok_or_else(|| format!("{file} evaluated"))?
            .to_string();
        assert!(error.contains(expected), "{file}: {error}");
    }
    Ok(())
}

/// A decorated notice output names the capture but carries text the notice
/// never renders, so only the exact placeholder is admitted.
#[test]
fn a_decorated_notice_output_is_refused() -> Result {
    let path = root()
        .join("invalid")
        .join("postgres-notice-decorated-output.pkl");
    let error = ruxel_config::eval_config(&path, &options())
        .err()
        .ok_or("evaluated")?
        .to_string();
    let expected = "a notice reads one capture's output exactly; read the report's stdout() with no added text";
    assert!(error.contains(expected), "{error}");
    Ok(())
}

/// An omitted port is the agent default, not another cluster: the same
/// resource with an unset port and with 5432 stated merges.
#[test]
fn an_unset_port_is_the_default_port() -> Result {
    let config = valid("postgres-role-implicit-port.pkl")?;
    let all = tasks(&config);
    assert_eq!(all.len(), 1, "{all:?}");
    Ok(())
}

/// An omitted table schema is the public schema the agent reads: the same
/// grant with an unset schema and with public stated merges.
#[test]
fn an_unset_table_schema_is_the_public_schema() -> Result {
    let config = valid("postgres-privileges-implicit-schema.pkl")?;
    let all = tasks(&config);
    assert_eq!(all.len(), 1, "{all:?}");
    Ok(())
}

/// Default privileges without target roles ride the login role's future
/// tables, so the same schema granted through two logins is two grants.
#[test]
fn default_privileges_without_targets_ride_the_login_role() -> Result {
    let config = valid("postgres-default-privs-login-target.pkl")?;
    let all = tasks(&config);
    assert_eq!(all.len(), 2, "{all:?}");
    Ok(())
}

/// An implied default-privilege target is the login role, so the same grant
/// with the target stated and implied is one declaration, not a conflict.
#[test]
fn default_privileges_with_stated_and_implied_targets_merge() -> Result {
    let config = valid("postgres-default-privs-implicit-target.pkl")?;
    let all = tasks(&config);
    assert_eq!(all.len(), 1, "{all:?}");
    Ok(())
}

/// A long `pg_hba` column of short names stays admitted: PostgreSQL truncates
/// identifiers, not columns, so each comma-separated member is checked
/// alone.
#[test]
fn a_long_pg_hba_list_of_short_names_is_admitted() -> Result {
    let config = valid("postgres-pg-hba-long-list.pkl")?;
    let all = tasks(&config);
    assert_eq!(all.len(), 2, "{all:?}");
    Ok(())
}

/// The same schemas in another order are one grant: the two declarations
/// merge instead of conflicting.
#[test]
fn same_objects_in_another_order_are_one_grant() -> Result {
    let config = valid("postgres-privileges-order.pkl")?;
    let all = tasks(&config);
    assert_eq!(all.len(), 1, "{all:?}");
    Ok(())
}

/// One host can run several clusters: the same role name on another port
/// is another thing, so the two declarations lower to two tasks.
#[test]
fn same_name_on_another_port_is_another_role() -> Result {
    let config = valid("postgres-role-two-ports.pkl")?;
    let all = tasks(&config);
    assert_eq!(all.len(), 2, "{all:?}");
    for task in &all {
        assert_eq!(task["operation"]["kind"], "postgres_role");
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
