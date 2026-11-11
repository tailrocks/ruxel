use super::super::{Backend, Lookup, ProcessBackend, Resolver};
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
fn command_lookup_passes_literal_arguments() {
    let fixture = Fixture::new();
    let command = fixture.script(
        "fake-command",
        "[ \"$#\" -eq 1 ] || exit 2; printf '%s' \"$1\"",
    );
    let instance = resolver(&command, &command);
    let value = "literal space; not shell syntax";
    assert_eq!(
        instance
            .resolve(&Lookup::Command {
                argv: vec![command.to_string_lossy().into_owned(), value.into()],
            })
            .unwrap(),
        value
    );
}

#[test]
fn synthetic_executables_exercise_arguments_dedup_and_redaction() {
    let fixture = Fixture::new();
    let op = fixture.script(
        "fake-op",
        // The fetch is field-scoped: `op item get --fields <field>`, and the
        // answer is the array of matching field objects.
        r#"[ "$#" -eq 10 ] && [ "$1" = item ] && [ "$2" = get ] && [ "$3" = --vault ] && [ "$4" = vault ] && [ "$5" = --fields ] && [ "$7" = --format ] && [ "$8" = json ] && [ "$9" = -- ] && [ "${10}" = --help ] || exit 4
printf 'call\n' >> "${0%/*}/calls"
case "$6" in
username) printf '%s' '[{"id":"username","label":"Username","value":"alice"}]';;
login.Password) printf '%s' '[{"label":"Password","section":{"id":"login","label":"Login"},"value":"synthetic-value"}]';;
*) exit 5;;
esac"#,
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
    // One op call per named field, shared by the repeated lookups.
    assert_eq!(fixture.count(), 2);
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
fn a_section_qualified_field_uses_the_cli_section_field_selector() {
    let fixture = Fixture::new();
    let op = fixture.script(
        "fake-op",
        r#"[ "$#" -eq 10 ] && [ "$1" = item ] && [ "$2" = get ] && [ "$3" = --vault ] && [ "$4" = vault ] && [ "$5" = --fields ] && [ "$6" = API.Password ] && [ "$7" = --format ] && [ "$8" = json ] && [ "$9" = -- ] && [ "${10}" = item ] || exit 4
printf 'call\n' >> "${0%/*}/calls"
printf '%s' '[{"id":"password","label":"Password","section":{"id":"api","label":"API"},"value":"fake-password-api"}]'"#,
    );
    let instance = resolver(&op, &op);
    let lookup = Lookup::OnePassword {
        vault: "vault".into(),
        item: "item".into(),
        field: "Password".into(),
        section: Some("API".into()),
    };
    assert_eq!(instance.resolve(&lookup).unwrap(), "fake-password-api");
    assert_eq!(fixture.count(), 1);
}

#[test]
fn op_fields_with_cli_selector_syntax_are_refused_before_op_runs() {
    let fixture = Fixture::new();
    // The fake op records every call and fails: nothing may reach it.
    let op = fixture.script("fake-op", "printf 'call\\n' >> \"${0%/*}/calls\"\nexit 9");
    let instance = resolver(&op, &op);
    // A field is passed to `op item get --fields`, which parses `,` as a
    // list separator and `=` as `label=`/`type=` selectors, so a legal
    // OpLabel like "API,key" would fetch unintended fields.
    let refusal = "may not contain a comma or an equals sign: `op item get --fields` reads them as selectors, so it would fetch unintended fields";
    for field in ["API,key", "label=Password"] {
        assert_eq!(
            instance
                .resolve(&Lookup::OnePassword {
                    vault: "vault".into(),
                    item: "item".into(),
                    field: field.into(),
                    section: None,
                })
                .unwrap_err(),
            format!("1Password field \"{field}\" {refusal}")
        );
    }
    // The op:// reference form reaches the same refusal through its field.
    assert_eq!(
        instance
            .resolve(&Lookup::OnePasswordUri {
                reference: "op://vault/item/API,key".into(),
            })
            .unwrap_err(),
        format!("1Password field \"API,key\" {refusal}")
    );
    // A section is the other component of the CLI's `section.field`
    // selector, so it cannot carry the same selector punctuation.
    assert_eq!(
        instance
            .resolve(&Lookup::OnePassword {
                vault: "vault".into(),
                item: "item".into(),
                field: "Password".into(),
                section: Some("API,key".into()),
            })
            .unwrap_err(),
        "1Password section \"API,key\" may not contain a comma or an equals sign: `op item get --fields` reads them as selectors, so it would fetch unintended fields"
    );
    // A dot is the CLI's section separator: a field literally labeled
    // "API.Password" would be fetched as section "API", field "Password",
    // a different selector than the author named, and a dotted section
    // would split the same way.
    let dotted = "may not contain a dot: `op item get --fields` reads a dot as the section separator, so it would fetch a different selector than the author named";
    assert_eq!(
        instance
            .resolve(&Lookup::OnePassword {
                vault: "vault".into(),
                item: "item".into(),
                field: "API.Password".into(),
                section: None,
            })
            .unwrap_err(),
        format!("1Password field \"API.Password\" {dotted}")
    );
    assert_eq!(
        instance
            .resolve(&Lookup::OnePasswordUri {
                reference: "op://vault/item/API.Password".into(),
            })
            .unwrap_err(),
        format!("1Password field \"API.Password\" {dotted}")
    );
    assert_eq!(
        instance
            .resolve(&Lookup::OnePassword {
                vault: "vault".into(),
                item: "item".into(),
                field: "Password".into(),
                section: Some("A.B".into()),
            })
            .unwrap_err(),
        format!("1Password section \"A.B\" {dotted}")
    );
    // The refusal happens at resolution, before any process runs.
    assert_eq!(fixture.count(), 0);
}

/// These CLIs append exactly one record terminator to the value: printenv
/// under fnox, and `op read`. A value that legitimately ends in a newline —
/// PEM keys do — must keep it, so the terminator alone is removed, never a
/// newline the value itself carries.
#[test]
fn only_the_record_terminator_is_removed_not_value_newlines() {
    let fixture = Fixture::new();
    let instance = resolver(
        &fixture.script("fake-fnox-keep", "printf 'line1\\n\\n'"),
        &fixture.script("fake-op-keep", "printf 'line1\\n\\n'"),
    );
    assert_eq!(
        instance
            .resolve(&Lookup::Fnox { key: "KEY".into() })
            .unwrap(),
        "line1\n"
    );
    assert_eq!(
        instance
            .resolve(&Lookup::OnePasswordUri {
                reference: "op://vault/item/password?attribute=pem".into(),
            })
            .unwrap(),
        "line1\n"
    );

    // One terminator, spelled \r\n, is removed whole: the value is empty of
    // newlines afterwards.
    let instance = resolver(
        &fixture.script("fake-fnox-crlf", "printf 'line1\\r\\n'"),
        &fixture.script("fake-op-crlf", "printf 'line1\\r\\n'"),
    );
    assert_eq!(
        instance
            .resolve(&Lookup::Fnox { key: "KEY".into() })
            .unwrap(),
        "line1"
    );

    // A value that already ends in \r\n keeps it when the terminator joins:
    // only the terminator is the CLI's addition.
    let instance = resolver(
        &fixture.script("fake-fnox-crlf-keep", "printf 'line1\\r\\n\\n'"),
        &fixture.script("fake-op-crlf-keep", "printf 'line1\\r\\n\\n'"),
    );
    assert_eq!(
        instance
            .resolve(&Lookup::Fnox { key: "KEY".into() })
            .unwrap(),
        "line1\r\n"
    );
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
    let lookup = Lookup::OnePasswordUri {
        reference: "op://vault/item/username?attribute=otp".into(),
    };
    assert_eq!(instance.resolve(&lookup).unwrap(), "synthetic-otp");
    assert_eq!(instance.resolve(&lookup).unwrap(), "synthetic-otp");
    assert_eq!(fixture.count(), 1);
    let invalid = Lookup::OnePasswordUri {
        reference: "op://vault/item/username?attribute=synthetic-invalid".into(),
    };
    for _ in 0..2 {
        assert_eq!(
            instance.resolve(&invalid).unwrap_err(),
            "secret lookup failed"
        );
    }
    assert_eq!(fixture.count(), 3);
}

#[test]
fn fnox_keys_follow_the_store_grammar() {
    let backend = ProcessBackend::with_programs("/nonexistent/ruxel-fnox", "/nonexistent/ruxel-op");
    // Keys outside the store's grammar are refused before any process starts,
    // with the same sentence Core and Lowering use.
    for key in [
        "",
        "a",
        "_",
        "_KEY",
        "key123",
        "app_secret",
        "KEY-NAME",
        "KEY NAME",
    ] {
        let error = backend.fnox(key).unwrap_err();
        assert!(
            error.contains(
                "is malformed: uppercase letters, digits and underscores only, starting with a letter, 2 to 128 characters",
            ),
            "{key:?}: {error}"
        );
    }
    // A key of the store's grammar reaches the program: the failure is the
    // missing process, not the key.
    assert_eq!(backend.fnox("KEY").unwrap_err(), "process could not start");
    assert_eq!(
        backend.fnox("KEY_2").unwrap_err(),
        "process could not start"
    );
}
