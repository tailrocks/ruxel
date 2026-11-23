use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

const FAKE_GIT: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$GIT_TEST_TRACE"
if [ "$1" = rev-parse ] && [ "$2" = HEAD ]; then
  count=0
  if [ -f "$GIT_TEST_COUNT" ]; then read -r count < "$GIT_TEST_COUNT"; fi
  count=$((count + 1))
  printf '%s\n' "$count" > "$GIT_TEST_COUNT"
  if [ "$count" = "$GIT_TEST_FAIL_AT" ]; then printf 'HEAD\n'; exit 128; fi
  printf '%040d\n' 1
elif [ "$1" = ls-remote ]; then
  printf '%040d\tHEAD\n' 1
fi
"#;

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn failed_head_resolution_never_reports_success() {
    exercise(1);
    exercise(2);
}

#[test]
fn successful_head_resolution_preserves_results() {
    exercise(0);
}

fn exercise(fail_at: u8) {
    for (case, (exists, update, check_mode)) in [
        (false, true, false),
        (true, false, false),
        (true, false, true),
        (true, true, false),
        (true, true, true),
    ]
    .into_iter()
    .enumerate()
    {
        let root = std::env::temp_dir().join(format!(
            "ruxel-git-head-{}-{fail_at}-{case}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let fixture = Fixture(root);
        let root = &fixture.0;
        let dest = root.join("dest");
        std::fs::create_dir(&dest).unwrap();
        if exists {
            std::fs::create_dir(dest.join(".git")).unwrap();
        }
        let fake = root.join("git");
        std::fs::write(&fake, FAKE_GIT).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let trace = root.join("trace");
        let ctx = super::ExecContext {
            check_mode,
            diff_mode: false,
            no_log: false,
            become_user: None,
            environment: vec![
                ("PATH".into(), root.to_string_lossy().into_owned()),
                (
                    "GIT_TEST_TRACE".into(),
                    trace.to_string_lossy().into_owned(),
                ),
                (
                    "GIT_TEST_COUNT".into(),
                    root.join("count").to_string_lossy().into_owned(),
                ),
                ("GIT_TEST_FAIL_AT".into(), fail_at.to_string()),
            ],
        };
        let result = super::run(
            &serde_json::json!({"repo":"synthetic-repository", "dest":dest, "update":update}),
            &ctx,
        );
        let fails = fail_at == 1 || (fail_at == 2 && exists && update && !check_mode);
        let calls = std::fs::read_to_string(trace).unwrap();
        if fails {
            if exists {
                assert_eq!(result.unwrap_err(), "git rev-parse HEAD failed");
            } else {
                let value = result.unwrap();
                assert_eq!(value["failed"], true);
                assert_eq!(value["changed"], true);
                assert_eq!(value["msg"], "git rev-parse HEAD failed");
            }
            assert!(calls.ends_with("rev-parse HEAD\n"));
            if fail_at == 1 && exists {
                assert_eq!(calls, "rev-parse HEAD\n");
            }
        } else {
            let value = result.unwrap();
            assert_eq!(value["failed"], false);
            assert_eq!(value["after"], format!("{:040}", 1));
            if exists {
                assert_eq!(value["before"], value["after"]);
                assert_eq!(value["changed"], false);
            } else {
                assert!(value["before"].is_null());
                assert_eq!(value["changed"], true);
            }
        }
    }
}
