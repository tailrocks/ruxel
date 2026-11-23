use super::trusted_executable;
use crate::modules::get_url::download_tests::Root;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

#[test]
fn system_executable_resolves_to_an_absolute_trusted_path() {
    let executable = trusted_executable(Path::new("/bin/sh")).unwrap();
    assert!(executable.is_absolute());
    assert!(executable.is_file());
}

#[test]
fn writable_or_user_owned_client_and_symlink_location_are_rejected() {
    let root = Root::new();
    let executable = root.0.join("client");
    fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(trusted_executable(&executable).is_err());
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(trusted_executable(&executable).is_err());
    let link = root.0.join("link");
    symlink("/bin/sh", &link).unwrap();
    assert!(trusted_executable(&link).is_err());
}
