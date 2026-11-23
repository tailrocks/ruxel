use super::*;
use std::os::unix::ffi::OsStringExt;

#[test]
fn ssh_path_options_quote_spaces_and_preserve_non_utf8_bytes() {
    let mut host = spec();
    host.identity_file = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/key \xff\"".to_vec()));
    host.known_hosts_file = PathBuf::from("/tmp/known hosts\\file");
    let args = resolve::build_args(&host, &PathBuf::from("/tmp/mux"), ArgsMode::Normal);
    assert!(args.contains(&std::ffi::OsString::from_vec(
        b"IdentityFile=\"/tmp/key \xff\\\"\"".to_vec()
    )));
    assert!(args.contains(&std::ffi::OsString::from(
        "UserKnownHostsFile=\"/tmp/known hosts\\\\file\""
    )));
}

#[test]
fn ssh_paths_reject_expansion_controls_and_relative_names() {
    for path in ["relative/key", "/tmp/%h", "/tmp/${HOME}", "/tmp/key\n"] {
        let mut host = spec();
        host.identity_file = PathBuf::from(path);
        assert!(resolve::validate_paths(&host).is_err());
        host.identity_file = PathBuf::from("/tmp/key");
        host.known_hosts_file = PathBuf::from(path);
        assert!(resolve::validate_paths(&host).is_err());
    }
    assert!(resolve::validate_paths(&spec()).is_ok());
}
