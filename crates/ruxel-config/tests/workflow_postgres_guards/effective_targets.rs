//! A sibling of `workflow_postgres_guards.rs`: the effective defaults the
//! lowering fills in — an omitted port is the agent default, an omitted
//! table schema is the public schema, and default privileges ride the login
//! role unless their targets are stated.

use super::*;

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
