use super::*;

fn context(check_mode: bool) -> ExecContext {
    ExecContext {
        check_mode,
        diff_mode: false,
        no_log: false,
        environment: vec![("FIXTURE".into(), "value".into())],
        become_user: None,
    }
}

fn summary(changed: bool) -> String {
    format!(
        "0 upgraded, 0 newly installed, {} to remove and 0 not upgraded.\n",
        u8::from(changed)
    )
}

mod cache;
mod maintenance;
mod process;
