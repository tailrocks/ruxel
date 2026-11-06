use super::*;

#[test]
fn acl_probes_expand_all_and_normalize_temp() {
    for (kind, held) in [
        ("database", vec!["CONNECT", "CREATE", "TEMPORARY"]),
        ("schema", vec!["CREATE", "USAGE"]),
        (
            "table",
            vec![
                "DELETE",
                "INSERT",
                "REFERENCES",
                "SELECT",
                "TRIGGER",
                "TRUNCATE",
                "UPDATE",
            ],
        ),
        (
            "default_privs",
            vec![
                "DELETE",
                "INSERT",
                "REFERENCES",
                "SELECT",
                "TRIGGER",
                "TRUNCATE",
                "UPDATE",
            ],
        ),
    ] {
        for requested in ["ALL", "all privileges"] {
            let probes = super::acl_privileges(requested, kind, 160_000);
            assert_eq!(probes, held);
            assert!(
                probes
                    .iter()
                    .all(|privilege| held.contains(&privilege.as_str()))
            );
            assert!(
                probes
                    .iter()
                    .any(|privilege| !held[1..].contains(&privilege.as_str()))
            );
        }
    }
    assert_eq!(
        super::acl_privileges("temp, TEMPORARY", "database", 180_000),
        ["TEMPORARY"]
    );
}

#[test]
fn all_table_acl_probes_follow_server_version() {
    for kind in ["table", "default_privs"] {
        let old = super::acl_privileges("ALL", kind, 160_000);
        let current = super::acl_privileges("ALL PRIVILEGES", kind, 170_000);
        assert!(!old.iter().any(|p| p == "MAINTAIN"));
        assert!(current.iter().any(|p| p == "MAINTAIN"));
        assert_eq!(current.len(), old.len() + 1);
        assert!(current.iter().any(|p| !old.contains(p)));
        assert!(super::acl_privileges("all", kind, 180_000).contains(&"MAINTAIN".into()));
    }
    assert!(super::validate_privs("MAINTAIN").is_ok());
}

#[test]
fn b64_roundtrip() {
    for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
        let enc = super::b64_encode(s.as_bytes());
        assert_eq!(super::b64_decode(&enc).unwrap(), s.as_bytes());
    }
}

#[test]
fn sql_inputs_are_allowlisted_and_quoted() {
    assert!(validate_privs("SELECT,UPDATE").is_ok());
    assert!(validate_privs("SELECT; DROP TABLE users").is_err());
    assert_eq!(lit("it's"), "'it''s'");
    assert_eq!(ident("odd\"name"), "\"odd\"\"name\"");
}

#[test]
fn grants_and_default_acl_queries_target_role_and_schema() {
    let sql = grant_sql(
        "table",
        "reader",
        "SELECT",
        Some("events"),
        Some("audit"),
        None,
    )
    .unwrap();
    assert_eq!(
        sql,
        vec!["GRANT SELECT ON TABLE \"audit\".\"events\" TO \"reader\""]
    );
    let query = default_priv_query("reader", "audit", "select", None);
    assert!(query.contains("n.nspname='audit'"));
    assert!(query.contains("rolname='reader'"));
    assert!(query.contains("privilege_type='SELECT'"));
}

#[test]
fn default_privileges_target_object_creator_roles() {
    let query = default_priv_query("reader", "audit", "select", Some("writer"));
    assert!(query.contains("d.defaclrole=(SELECT oid FROM pg_roles WHERE rolname='writer')"));
    let sql = grant_sql(
        "default_privs",
        "reader",
        "SELECT",
        Some("TABLES"),
        Some("audit"),
        Some("writer,loader"),
    )
    .unwrap();
    assert_eq!(sql.len(), 2);
    assert!(sql[0].contains("FOR ROLE \"writer\""));
    assert!(sql[1].contains("FOR ROLE \"loader\""));
}
