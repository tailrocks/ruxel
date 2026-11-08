use super::super::*;

#[test]
fn role_flag_wants_positive_and_negative_forms() {
    assert_eq!(wanted_flag("LOGIN,CREATEDB", "LOGIN"), Some(true));
    assert_eq!(wanted_flag("NOLOGIN", "LOGIN"), Some(false));
    assert_eq!(wanted_flag("CREATEDB", "LOGIN"), None);
}

#[test]
fn scram_stored_key_matches_postgres() {
    let got = super::scram_stored_key("s3cr3t-ruxel", "tmEKTUhvHycriHKDSR74nA==", 4096).unwrap();
    assert_eq!(got, "qsXMYQ6PkvDbKdO/Fwo5aAQisdU9bG3fdoLEpQMpraM=");
}

#[test]
fn scram_unicode_and_raw_fallback_match_postgres_verifiers() {
    assert_eq!(
        super::scram_stored_key("I\u{ad}X", "NRomlZ/76mInmJVcVAWwbQ==", 4096).unwrap(),
        "LQcBrYvWXrPnADjvh/bqaQznzfnWXeocK5vlJKvuT4E="
    );
    assert_eq!(
        super::scram_stored_key("\u{1d2c}", "fMo3oNP3ejrm88d6dVe1HA==", 4096).unwrap(),
        "YcdRqCSXlnK+KcNcyiufqcvBwdBIA5oz9RSWDGC6FDs="
    );
}

#[test]
fn b64_decode_partial_groups() {
    assert_eq!(super::b64_decode("nA==").unwrap(), vec![0x9c]);
    assert_eq!(super::b64_decode("tA==").unwrap().len(), 1);
}

#[test]
fn role_attribute_allowlist_accepts_boolean_flags() {
    assert!(validate_role_attr_flags("SUPERUSER,CREATEDB").is_ok());
    assert!(validate_role_attr_flags("NOLOGIN").is_ok());
    assert!(validate_role_attr_flags("NOLOGIN,login").is_err());
    assert!(validate_role_attr_flags("CREATEDB,NOCREATEDB").is_err());
}

#[test]
fn role_attribute_allowlist_rejects_statement_injection() {
    assert!(validate_role_attr_flags("SUPERUSER; DROP ROLE synthetic").is_err());
    assert!(validate_role_attr_flags("CONNECTION LIMIT 5").is_err());
}

#[test]
fn wanted_role_flag_handles_positive_negative_and_absent() {
    assert_eq!(wanted_flag("CREATEDB", "CREATEDB"), Some(true));
    assert_eq!(wanted_flag("NOCREATEDB", "CREATEDB"), Some(false));
    assert_eq!(wanted_flag("LOGIN", "CREATEDB"), None);
}

#[test]
fn default_privilege_query_targets_explicit_table_acl() {
    let query = default_priv_query("synthetic_role", "public", "select", None);
    assert!(query.contains("pg_default_acl"));
    assert!(query.contains("aclexplode(d.defaclacl)"));
    assert!(query.contains("d.defaclobjtype='r'"));
    assert!(query.contains("a.grantee=(SELECT oid FROM pg_roles"));
    assert!(query.contains("a.privilege_type='SELECT'"));
}
