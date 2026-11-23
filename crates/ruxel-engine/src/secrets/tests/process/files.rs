//! The `files` contract of `fnox env --json` for `as_file = true` keys:
//! a file-backed key resolves to the raw contents the store returns, and
//! no decrypted plaintext is left on disk after the lookup. `fnox get`
//! creates a persistent temporary file that holds the value and prints
//! its path; the store's own query names file-backed keys before any
//! `get` runs, and the file `get` names is removed on every path.

use super::super::super::Lookup;
use super::{Fixture, resolver};

/// A file-backed key resolves to the raw contents the store returns in
/// the `files` entry of `fnox env --json --keys`, which leaves no files
/// behind — the caller writes them. The store's own query reports the
/// key's `as_file` flag first, so the read never runs `fnox get`, whose
/// persistent file would hold the decrypted value on disk.
#[test]
fn a_file_backed_secret_resolves_to_the_file_contents_of_the_files_contract() {
    let fixture = Fixture::new();
    let fnox = fixture.script(
        "fake-fnox",
        r#"case "$1" in
get)
  # `fnox get` prints the path of a persistent file that holds the
  # decrypted value — plaintext left on disk. The `files` contract serves
  # this key and leaves no files behind, so `get` must never run for it.
  printf 'call\n' >> "${0%/*}/calls"
  printf 'fnox-get-must-not-run-for-a-file-backed-key' >&2; exit 2;;
env)
  [ "$2" = --json ] || exit 6
  case "$3" in
  --describe)
    # The unfiltered query lists every key with its `env` and `as_file`
    # flags: a file-backed key is known before any `get` runs.
    [ "$#" -eq 3 ] || exit 6
    printf '%s' '{"schema":1,"keys":[{"key":"FILE_KEY","kind":"secret","env":true,"as_file":true}]}';;
  --keys)
    # The `files` entry carries the raw contents and leaves no files
    # behind.
    [ "$#" -eq 4 ] && [ "$4" = FILE_KEY ] || exit 6
    printf '%s' '{"schema":1,"set":{},"files":{"FILE_KEY":"the-secret-content"},"remove":[],"missing":[],"leases":[]}';;
  *) exit 6;;
  esac;;
*) exit 9;;
esac"#,
    );
    let instance = resolver(&fnox, &fnox);
    assert_eq!(
        instance
            .resolve(&Lookup::Fnox {
                key: "FILE_KEY".into(),
            })
            .unwrap(),
        "the-secret-content"
    );
    assert_eq!(fixture.count(), 0, "fnox get ran for a file-backed key");
}

/// After the lookup the file the store named for a file-backed key does
/// not exist on disk: `fnox get` prints the path of a persistent file
/// that holds the decrypted value, and that file is removed on every
/// path — read success, read error, and any early return. A key
/// configured `env = false` reaches the fallback, because the `files`
/// contract of `fnox env --json --keys` refuses it as not injectable.
#[test]
fn after_the_lookup_the_file_the_store_named_does_not_exist_on_disk() {
    let fixture = Fixture::new();
    let fnox = fixture.script(
        "fake-fnox",
        r#"case "$1" in
get)
  [ "$#" -eq 4 ] && [ "$2" = --if-missing ] && [ "$3" = error ] || exit 6
  case "$4" in
  ENV_FALSE_FILE)
    # The upstream CLI creates a persistent file that holds the
    # decrypted value and prints its path — decrypted plaintext on disk.
    printf '%s' 'the-secret-content' > "${0%/*}/held-value"
    printf '%s\n' "${0%/*}/held-value";;
  *) printf 'fnox-sensitive-missing-error' >&2; exit 7;;
  esac;;
env)
  [ "$2" = --json ] || exit 6
  case "$3" in
  --describe)
    # The unfiltered query reports the key's `env` and `as_file` flags
    # even though `--keys` refuses an env=false key.
    [ "$#" -eq 3 ] || exit 6
    printf '%s' '{"schema":1,"keys":[{"key":"ENV_FALSE_FILE","kind":"secret","env":false,"as_file":true}]}';;
  --keys)
    # The preferred contract cannot serve an env=false key: it is
    # refused as not injectable, so the read falls back to `get`.
    [ "$#" -eq 4 ] && [ "$4" = ENV_FALSE_FILE ] || exit 6
    printf '%s' '{"schema":1,"error":{"kind":"invalid_keys","message":"fnox env cannot provide: not injectable: ENV_FALSE_FILE","not_injectable":[{"key":"ENV_FALSE_FILE","env":false}]}}'; exit 1;;
  *) exit 6;;
  esac;;
*) exit 9;;
esac"#,
    );
    let instance = resolver(&fnox, &fnox);
    assert_eq!(
        instance
            .resolve(&Lookup::Fnox {
                key: "ENV_FALSE_FILE".into(),
            })
            .unwrap(),
        "the-secret-content"
    );
    let named = fixture.0.join("held-value");
    assert!(
        !named.exists(),
        "the file the store named still holds decrypted plaintext on disk: {}",
        named.display()
    );
}
