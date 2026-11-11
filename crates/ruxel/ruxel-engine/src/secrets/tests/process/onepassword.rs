//! The 1Password lookups of the resolver against synthetic `op`
//! executables: the calls the CLI is driven with, the refusals that happen
//! before it runs, and the redaction of what it answers.

use super::super::super::Lookup;
use super::{Fixture, fake_fnox_printing, resolver};

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
note) printf '%s' '[{"id":"note","label":"NOTE","value":"process-note"}]';;
*) exit 5;;
esac"#,
    );
    let fnox = fixture.script(
        "fake-fnox",
        // `get` prints the value; `env --json --describe` answers the
        // store's own question about the key the way the CLI does: the
        // unfiltered query lists every key with its `env` and `as_file`
        // flags, and `--keys` names only injectable keys.
        r#"case "$1" in
get)
  [ "$#" -eq 4 ] && [ "$2" = --if-missing ] && [ "$3" = error ] && [ "$4" = KEY ] || exit 5
  printf synthetic-fnox;;
env)
  [ "$2" = --json ] && [ "$3" = --describe ] || exit 5
  if [ "$#" -eq 5 ] && [ "$4" = --keys ] && [ "$5" = KEY ]; then
    printf '%s' '{"schema":1,"keys":[{"key":"KEY","kind":"secret","env":true,"as_file":false}]}'
  elif [ "$#" -eq 3 ]; then
    printf '%s' '{"schema":1,"keys":[{"key":"KEY","kind":"secret","env":true,"as_file":false}]}'
  else exit 5; fi;;
*) exit 9;;
esac"#,
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
    // The reference's case reaches `--fields` as authored; selection folds
    // it against the field labeled "NOTE".
    let note = Lookup::OnePasswordUri {
        reference: "op://vault/--help/note".into(),
    };
    instance
        .prefetch(&[
            username.clone(),
            password.clone(),
            note.clone(),
            username.clone(),
        ])
        .unwrap();
    assert_eq!(instance.resolve(&username).unwrap(), "alice");
    assert_eq!(instance.resolve(&password).unwrap(), "synthetic-value");
    assert_eq!(instance.resolve(&note).unwrap(), "process-note");
    // One op call per named field, shared by the repeated lookups.
    assert_eq!(fixture.count(), 3);
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
    // would split the same way. The structured selector refuses the dot.
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

/// The parts of a reference are separated by `/`, so a dot is an ordinary
/// character of a label there: `op://vault/item/API.Password` names the
/// field labeled `API.Password`. The reference path reads it, because the
/// `--fields` selector cannot spell it.
#[test]
fn a_dotted_reference_is_read_through_the_reference_path() {
    let fixture = Fixture::new();
    let op = fixture.script(
        "fake-op",
        r#"[ "$#" -eq 3 ] && [ "$1" = read ] && [ "$2" = -- ] || exit 4
printf 'call\n' >> "${0%/*}/calls"
case "$3" in
'op://vault/item/API.Password') printf 'dotted-value';;
*) exit 5;;
esac"#,
    );
    let instance = resolver(&op, &op);
    assert_eq!(
        instance
            .resolve(&Lookup::OnePasswordUri {
                reference: "op://vault/item/API.Password".into(),
            })
            .unwrap(),
        "dotted-value"
    );
    assert_eq!(fixture.count(), 1);
}

/// The reference components are validated before the dotted-reference path
/// is chosen: `op read` names the reference as it is written, so a dot in a
/// label is fine there, but a comma or an equals sign is still the CLI's
/// selector punctuation and is refused before either read path runs. The
/// accepted forms keep their routes: a plain reference through the field
/// lookup, a dotted one through the reference path.
#[test]
fn selector_punctuation_is_refused_before_a_dotted_reference_is_routed() {
    let fixture = Fixture::new();
    let op = fixture.script(
        "fake-op",
        r#"printf 'call\n' >> "${0%/*}/calls"
case "$1" in
read)
  [ "$#" -eq 3 ] && [ "$2" = -- ] || exit 4
  [ "$3" = 'op://vault/item/API.Password' ] || exit 5
  printf 'dotted-value';;
item)
  [ "$#" -eq 10 ] && [ "$2" = get ] && [ "$5" = --fields ] && [ "$6" = username ] || exit 4
  printf '%s' '[{"id":"username","label":"Username","value":"alice"}]';;
*) exit 9;;
esac"#,
    );
    let instance = resolver(&op, &op);
    // The same sentence `op_field` refuses a direct selector with: the
    // components of a reference reach `op item get --fields` selectors or
    // a reference `op read` names, so the CLI's selector punctuation is
    // forbidden either way.
    let refusal = "may not contain a comma or an equals sign: `op item get --fields` reads them as selectors, so it would fetch unintended fields";
    for (reference, named) in [
        (
            "op://vault/item/API.Password,key",
            format!("1Password field \"API.Password,key\" {refusal}"),
        ),
        (
            "op://vault/item/API.Password=type",
            format!("1Password field \"API.Password=type\" {refusal}"),
        ),
        (
            "op://vault/item/API.Section,Name/Password",
            format!("1Password section \"API.Section,Name\" {refusal}"),
        ),
    ] {
        assert_eq!(
            instance
                .resolve(&Lookup::OnePasswordUri {
                    reference: reference.into(),
                })
                .unwrap_err(),
            named
        );
    }
    // The refusals happen before anything runs.
    assert_eq!(fixture.count(), 0);
    // The accepted forms still route to their own read paths.
    assert_eq!(
        instance
            .resolve(&Lookup::OnePasswordUri {
                reference: "op://vault/item/username".into(),
            })
            .unwrap(),
        "alice"
    );
    assert_eq!(
        instance
            .resolve(&Lookup::OnePasswordUri {
                reference: "op://vault/item/API.Password".into(),
            })
            .unwrap(),
        "dotted-value"
    );
    assert_eq!(fixture.count(), 2);
}

/// These CLIs append exactly one record terminator to the value: `fnox get`,
/// and `op read`. A value that legitimately ends in a newline — PEM keys
/// do — must keep it, so the terminator alone is removed, never a newline
/// the value itself carries.
#[test]
fn only_the_record_terminator_is_removed_not_value_newlines() {
    let fixture = Fixture::new();
    let instance = resolver(
        &fake_fnox_printing(&fixture, "fake-fnox-keep", "line1\\n\\n"),
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
        &fake_fnox_printing(&fixture, "fake-fnox-crlf", "line1\\r\\n"),
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
        &fake_fnox_printing(&fixture, "fake-fnox-crlf-keep", "line1\\r\\n\\n"),
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
