use super::*;

#[test]
fn privilege_requests_reject_empty_privileges_and_targets() {
    let ctx = ExecContext {
        check_mode: true,
        diff_mode: false,
        no_log: false,
        environment: Vec::new(),
        become_user: None,
    };
    for params in [
        json!({"login_db":"db", "role":"r", "type":"database", "objs":["db"]}),
        json!({"login_db":"db", "role":"r", "type":"table", "objs":["t"], "privs":[]}),
        json!({"login_db":"db", "role":"r", "type":"schema", "objs":[], "privs":["USAGE"]}),
        json!({"login_db":"db", "role":"r", "type":"table", "privs":["SELECT"]}),
        json!({"login_db":"db", "role":"r", "type":"table", "objs":["t"], "privs":["EXECUTE"]}),
        json!({"login_db":"db", "role":"r", "type":"database", "objs":["db"], "privs":["SELECT"]}),
        json!({"login_db":"db", "role":"r", "type":"default_privs", "privs":["SELECT"], "target_roles":[]}),
    ] {
        assert!(
            super::super::privileges::privs(&params, &ctx).is_err(),
            "{params}"
        );
    }
}
