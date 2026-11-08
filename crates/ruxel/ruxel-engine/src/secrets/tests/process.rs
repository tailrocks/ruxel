use super::super::{Lookup, ProcessBackend, Resolver};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ruxel-secret-process-{}-{}-'",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn count(&self) -> usize {
        fs::read_to_string(self.0.join("calls")).map_or(0, |calls| calls.lines().count())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _result = fs::remove_dir_all(&self.0);
    }
}

fn resolver(fnox: &Path, op: &Path) -> Resolver {
    Resolver::new(Arc::new(ProcessBackend::with_programs(fnox, op)))
}

#[test]
fn synthetic_executables_exercise_arguments_dedup_and_redaction() {
    let fixture = Fixture::new();
    let op = fixture.script(
        "fake-op",
        r#"[ "$#" -eq 8 ] && [ "$1" = item ] && [ "$2" = get ] && [ "$3" = --vault ] && [ "$4" = vault ] && [ "$5" = --format ] && [ "$6" = json ] && [ "$7" = -- ] && [ "$8" = --help ] || exit 4
printf 'call\n' >> "${0%/*}/calls"
printf '%s' '{"fields":[{"id":"username","value":"alice"},{"label":"Password","section":{"id":"login"},"value":"synthetic-value"}]}'"#,
    );
    let fnox = fixture.script(
        "fake-fnox",
        r#"[ "$#" -eq 10 ] && [ "$1" = exec ] && [ "$2" = --no-redact ] && [ "$3" = --only ] && [ "$4" = KEY ] && [ "$5" = -- ] && [ "$6" = sh ] && [ "$7" = -c ] && [ "$8" = 'printenv -- "$1"' ] && [ "$9" = sh ] && [ "${10}" = KEY ] || exit 5
printf synthetic-fnox"#,
    );
    let instance = resolver(&fnox, &op);
    let username = Lookup::OnePasswordUri {
        reference: "op://vault/--help/username".into(),
    };
    let password = Lookup::OnePassword {
        vault: "vault".into(),
        item: "--help".into(),
        field: "Password".into(),
        section: Some("login".into()),
    };
    instance
        .prefetch(&[username.clone(), password.clone(), username.clone()])
        .unwrap();
    assert_eq!(instance.resolve(&username).unwrap(), "alice");
    assert_eq!(instance.resolve(&password).unwrap(), "synthetic-value");
    assert_eq!(fixture.count(), 1);
    assert_eq!(
        instance
            .resolve(&Lookup::Fnox { key: "KEY".into() })
            .unwrap(),
        "synthetic-fnox"
    );

    let failing = fixture.script(
        "fake-failure",
        "printf synthetic-sensitive-error >&2; exit 7",
    );
    let failing_resolver = resolver(&failing, &failing);
    let error = failing_resolver
        .resolve(&Lookup::Fnox { key: "KEY".into() })
        .unwrap_err();
    assert_eq!(error, "secret lookup failed");
    assert!(!error.contains("synthetic-sensitive-error"));
}

#[test]
fn query_references_reach_mock_op_unchanged_and_failures_stay_redacted() {
    let fixture = Fixture::new();
    let op = fixture.script(
        "fake-op-query",
        r#"[ "$#" -eq 3 ] && [ "$1" = read ] && [ "$2" = -- ] || exit 4
printf 'call\n' >> "${0%/*}/calls"
case "$3" in
'op://vault/item/username?attribute=otp') printf synthetic-otp;;
*) printf synthetic-sensitive-query-error >&2; exit 7;;
esac"#,
    );
    let instance = resolver(&op, &op);
    for lookup in [
        Lookup::OnePasswordUri {
            reference: "op://vault/item/username?attribute=otp".into(),
        },
        Lookup::Pipe {
            command: "op read 'op://vault/item/username?attribute=otp'".into(),
        },
    ] {
        assert_eq!(instance.resolve(&lookup).unwrap(), "synthetic-otp");
        assert_eq!(instance.resolve(&lookup).unwrap(), "synthetic-otp");
    }
    assert_eq!(fixture.count(), 2);
    let invalid = Lookup::OnePasswordUri {
        reference: "op://vault/item/username?attribute=synthetic-invalid".into(),
    };
    for _ in 0..2 {
        assert_eq!(
            instance.resolve(&invalid).unwrap_err(),
            "secret lookup failed"
        );
    }
    assert_eq!(fixture.count(), 4);
}
