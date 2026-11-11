use super::super::{Backend, Lookup, ProcessBackend, Resolver};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "process/onepassword.rs"]
mod onepassword;

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

pub(super) struct Fixture(PathBuf);

impl Fixture {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ruxel-secret-process-{}-{}-'",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub(super) fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    pub(super) fn count(&self) -> usize {
        fs::read_to_string(self.0.join("calls")).map_or(0, |calls| calls.lines().count())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _result = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn resolver(fnox: &Path, op: &Path) -> Resolver {
    Resolver::new(Arc::new(ProcessBackend::with_programs(fnox, op))).unwrap()
}

/// A fake `fnox` that prints `printed` for every key and answers the
/// store's own question about the key the way the CLI does: `env --json
/// --describe` lists every key of the active profile with its `env` and
/// `as_file` flags, and `--keys` names only injectable keys.
pub(super) fn fake_fnox_printing(fixture: &Fixture, name: &str, printed: &str) -> PathBuf {
    let mut script = String::from("case \"$1\" in\nget) printf '");
    script.push_str(printed);
    script.push_str(
        "';;\nenv)\n  [ \"$2\" = --json ] && [ \"$3\" = --describe ] || exit 6\n\
         case \"$5\" in\n\
         '') printf '%s' '{\"schema\":1,\"keys\":[{\"key\":\"KEY\",\"kind\":\"secret\",\"env\":true,\"as_file\":false}]}';;\n\
         KEY) printf '%s' '{\"schema\":1,\"keys\":[{\"key\":\"KEY\",\"kind\":\"secret\",\"env\":true,\"as_file\":false}]}';;\n\
         *) printf '{\"schema\":1,\"error\":{\"kind\":\"invalid_keys\",\"message\":\"fnox env cannot provide: unknown keys: %s\",\"unknown\":[\"%s\"]}}' \"$5\" \"$5\"; exit 1;;\n\
         esac;;\n\
         *) exit 9;;\nesac",
    );
    fixture.script(name, &script)
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

/// A key configured `env = false` is never injected into `fnox exec`'s
/// environment, so an exec+printenv read always fails for it; the read goes
/// through `fnox get`, which prints the resolved value plus one newline
/// whatever the key's env setting. `--if-missing error` keeps a missing key
/// a failure: with the config's `warn` or `ignore`, `get` prints nothing and
/// exits 0, which would silently resolve to an empty value.
#[test]
fn env_false_keys_are_read_through_fnox_get_and_missing_keys_fail() {
    let fixture = Fixture::new();
    let fnox = fixture.script(
        "fake-fnox",
        r#"case "$1" in
get)
  [ "$#" -eq 4 ] && [ "$2" = --if-missing ] && [ "$3" = error ] || exit 6
  printf 'call\n' >> "${0%/*}/calls"
  case "$4" in
  ENV_FALSE_KEY) printf 'env=false-value\n';;
  *) printf 'fnox-sensitive-missing-error' >&2; exit 7;;
  esac;;
env)
  # The store's own question about the key, the way the CLI answers it:
  # the unfiltered query lists every key of the active profile with its
  # `env` and `as_file` flags, and `--keys` names only injectable keys —
  # an env=false key is refused with an invalid_keys error document.
  [ "$2" = --json ] && [ "$3" = --describe ] || exit 6
  case "$5" in
  '') printf '%s' '{"schema":1,"keys":[{"key":"ENV_FALSE_KEY","kind":"secret","env":false,"as_file":false}]}';;
  ENV_FALSE_KEY) printf '%s' '{"schema":1,"error":{"kind":"invalid_keys","message":"fnox env cannot provide: not injectable: ENV_FALSE_KEY","not_injectable":[{"key":"ENV_FALSE_KEY","env":false}]}}'; exit 1;;
  *) printf '{"schema":1,"error":{"kind":"invalid_keys","message":"fnox env cannot provide: unknown keys: %s","unknown":["%s"]}}' "$5" "$5"; exit 1;;
  esac;;
exec)
  # An env=false key is never injected, so printenv finds nothing.
  exit 1;;
*) exit 9;;
esac"#,
    );
    let instance = resolver(&fnox, &fnox);
    assert_eq!(
        instance
            .resolve(&Lookup::Fnox {
                key: "ENV_FALSE_KEY".into(),
            })
            .unwrap(),
        "env=false-value"
    );
    assert_eq!(fixture.count(), 1);
    // A missing key stays a redacted failure, never an empty value.
    let error = instance
        .resolve(&Lookup::Fnox {
            key: "MISSING".into(),
        })
        .unwrap_err();
    assert_eq!(error, "secret lookup failed");
    assert!(!error.contains("fnox-sensitive-missing-error"));
}

/// A secret configured `as_file = true` is read from the file the store
/// names: `fnox get` prints the path of a temporary file that holds the
/// value, and the value is that file's content. The store's own query says
/// which keys are file-backed, so the path is never taken for a secret.
#[test]
fn a_file_backed_secret_resolves_to_the_content_of_the_file_it_names() {
    let fixture = Fixture::new();
    let fnox = fixture.script(
        "fake-fnox",
        r#"case "$1" in
get)
  [ "$#" -eq 4 ] && [ "$2" = --if-missing ] && [ "$3" = error ] || exit 6
  case "$4" in
  FILE_KEY)
    printf '%s' 'the-secret-content' > "${0%/*}/held-value"
    printf '%s\n' "${0%/*}/held-value";;
  *) printf 'fnox-sensitive-missing-error' >&2; exit 7;;
  esac;;
env)
  # The store's own question about the key, the way the CLI answers it:
  # the unfiltered query lists every key with its `env` and `as_file`
  # flags, and `--keys` names only injectable keys.
  [ "$2" = --json ] && [ "$3" = --describe ] || exit 6
  case "$5" in
  '') printf '%s' '{"schema":1,"keys":[{"key":"FILE_KEY","kind":"secret","env":true,"as_file":true}]}';;
  FILE_KEY) printf '%s' '{"schema":1,"keys":[{"key":"FILE_KEY","kind":"secret","env":true,"as_file":true}]}';;
  *) printf '{"schema":1,"error":{"kind":"invalid_keys","message":"fnox env cannot provide: unknown keys: %s","unknown":["%s"]}}' "$5" "$5"; exit 1;;
  esac;;
*) exit 9;;
esac"#,
    );
    let instance = resolver(&fnox, &fnox);
    let value = instance
        .resolve(&Lookup::Fnox {
            key: "FILE_KEY".into(),
        })
        .unwrap();
    assert_eq!(value, "the-secret-content");
    assert!(
        !value.contains("held-value"),
        "the path of the file was taken for the secret: {value}"
    );
}

/// A key configured `env = false` and `as_file = true`: `fnox get` prints
/// the path of the temporary file that holds the value, and `fnox env
/// --json --describe` — the unfiltered query — reports the key's `as_file`
/// flag even though `--keys` refuses it as not injectable. The read goes
/// to the file the store names: the path is never taken for the secret.
#[test]
fn an_env_false_file_backed_key_resolves_to_the_file_content_not_the_pathname() {
    let fixture = Fixture::new();
    let fnox = fixture.script(
        "fake-fnox",
        r#"case "$1" in
get)
  [ "$#" -eq 4 ] && [ "$2" = --if-missing ] && [ "$3" = error ] || exit 6
  case "$4" in
  ENV_FALSE_FILE)
    printf '%s' 'the-secret-content' > "${0%/*}/held-value"
    printf '%s\n' "${0%/*}/held-value";;
  *) printf 'fnox-sensitive-missing-error' >&2; exit 7;;
  esac;;
env)
  # The real CLI contract: the unfiltered query lists every key of the
  # active profile with its `env` and `as_file` flags; `--keys` names only
  # injectable keys and refuses an env=false key with an invalid_keys
  # not_injectable error document.
  [ "$2" = --json ] && [ "$3" = --describe ] || exit 6
  case "$5" in
  '') printf '%s' '{"schema":1,"keys":[{"key":"ENV_FALSE_FILE","kind":"secret","env":false,"as_file":true}]}';;
  ENV_FALSE_FILE) printf '%s' '{"schema":1,"error":{"kind":"invalid_keys","message":"fnox env cannot provide: not injectable: ENV_FALSE_FILE","not_injectable":[{"key":"ENV_FALSE_FILE","env":false}]}}'; exit 1;;
  *) printf '{"schema":1,"error":{"kind":"invalid_keys","message":"fnox env cannot provide: unknown keys: %s","unknown":["%s"]}}' "$5" "$5"; exit 1;;
  esac;;
*) exit 9;;
esac"#,
    );
    let instance = resolver(&fnox, &fnox);
    let value = instance
        .resolve(&Lookup::Fnox {
            key: "ENV_FALSE_FILE".into(),
        })
        .unwrap();
    assert_eq!(value, "the-secret-content");
    assert!(
        !value.contains("held-value"),
        "the path of the file was taken for the secret: {value}"
    );
}

/// A store that cannot report whether a key is file-backed — it answers
/// the query with an error document — fails the read closed, naming
/// `as_file` as unsupported: the path `fnox get` printed is never returned
/// as a secret.
#[test]
fn a_store_that_answers_with_an_error_document_fails_the_read_closed() {
    let fixture = Fixture::new();
    let fnox = fixture.script(
        "fake-fnox",
        r#"case "$1" in
get)
  printf '%s\n' "${0%/*}/file-backed-value";;
env)
  printf '%s' '{"schema":1,"error":{"kind":"invalid_keys","message":"fnox env cannot provide: not injectable: KEY","not_injectable":[{"key":"KEY","env":false}]}}'; exit 1;;
*) exit 9;;
esac"#,
    );
    std::fs::write(fixture.0.join("held-value"), b"the-secret-content").unwrap();
    let backend = ProcessBackend::with_programs(&fnox, &fnox);
    let error = backend.fnox_get("KEY").unwrap_err();
    assert!(error.contains("as_file = true"), "{error}");
    assert!(error.contains("not supported"), "{error}");
    // The path the store printed is never the secret.
    assert!(!error.contains("file-backed-value"), "{error}");
}

/// A store that cannot say whether a key is file-backed fails the read,
/// naming `as_file` as unsupported: the path it printed is never returned
/// as a secret.
#[test]
fn a_store_that_cannot_report_file_backed_secrets_fails_the_read() {
    let fixture = Fixture::new();
    let fnox = fixture.script(
        "fake-fnox",
        r#"case "$1" in
get)
  printf '%s\n' "${0%/*}/file-backed-value";;
env)
  # A store without the query: it answers with something else.
  printf 'not a document\n';;
*) exit 9;;
esac"#,
    );
    std::fs::write(fixture.0.join("held-value"), b"the-secret-content").unwrap();
    let backend = ProcessBackend::with_programs(&fnox, &fnox);
    let error = backend.fnox_get("FILE_KEY").unwrap_err();
    assert!(error.contains("as_file = true"), "{error}");
    assert!(error.contains("not supported"), "{error}");
    // The path the store printed is never the secret.
    assert!(!error.contains("held-value"), "{error}");
}

/// A store that does not know the key answers the query with an error
/// document: the key is not file-backed, and the read fails with the
/// store's own missing-key error.
#[test]
fn an_unknown_key_is_not_file_backed_and_fails_the_read() {
    let fixture = Fixture::new();
    let fnox = fixture.script(
        "fake-fnox",
        r#"case "$1" in
get)
  printf 'fnox-sensitive-missing-error' >&2; exit 7;;
env)
  # The unfiltered query lists the keys of the active profile — an unknown
  # key is not among them — and `--keys` refuses it as unknown.
  [ "$2" = --json ] && [ "$3" = --describe ] || exit 6
  case "$5" in
  '') printf '%s' '{"schema":1,"keys":[]}';;
  *) printf '{"schema":1,"error":{"kind":"invalid_keys","message":"fnox env cannot provide: unknown keys: %s","unknown":["%s"]}}' "$5" "$5"; exit 1;;
  esac;;
*) exit 9;;
esac"#,
    );
    let backend = ProcessBackend::with_programs(&fnox, &fnox);
    assert_eq!(
        backend.fnox_get("MISSING").unwrap_err(),
        "process failed"
    );
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
        let error = backend.fnox_get(key).unwrap_err();
        assert!(
            error.contains(
                "is malformed: uppercase letters, digits and underscores only, starting with a letter, 2 to 128 characters",
            ),
            "{key:?}: {error}"
        );
    }
    // A key of the store's grammar reaches the program: the failure is the
    // missing process, not the key.
    assert_eq!(
        backend.fnox_get("KEY").unwrap_err(),
        "process could not start"
    );
    assert_eq!(
        backend.fnox_get("KEY_2").unwrap_err(),
        "process could not start"
    );
}
