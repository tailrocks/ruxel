//! A sibling of `workflow_postgres_guards.rs`: the 63-byte identifier
//! limit — overlong names are refused, while a long `pg_hba` column of
//! short names stays admitted because PostgreSQL truncates identifiers,
//! not columns.

use super::*;

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
