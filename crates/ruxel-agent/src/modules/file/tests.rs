#[path = "tests/absent.rs"]
mod absent;
#[path = "tests/attachment.rs"]
mod attachment;
#[path = "tests/attributes.rs"]
mod attributes;
#[path = "tests/empty_path.rs"]
mod empty_path;
#[path = "tests/extra.rs"]
mod extra;
#[path = "tests/floor.rs"]
mod floor;
#[path = "tests/mounts.rs"]
mod mounts;
#[path = "tests/path.rs"]
mod path;
#[path = "tests/removal.rs"]
mod removal;
#[path = "tests/root_alias.rs"]
mod root_alias;
#[path = "tests/tree.rs"]
mod tree;
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
