#[path = "tests/absent.rs"]
mod absent;
#[path = "tests/attributes.rs"]
mod attributes;
#[path = "tests/empty_path.rs"]
mod empty_path;
#[path = "tests/extra.rs"]
mod extra;
#[path = "tests/path.rs"]
mod path;
#[path = "tests/root_alias.rs"]
mod root_alias;
use super::*;
use std::os::unix::fs::PermissionsExt;

fn context() -> ExecContext {
    ExecContext {
        check_mode: false,
        diff_mode: false,
        no_log: false,
        environment: vec![],
        become_user: None,
    }
}
