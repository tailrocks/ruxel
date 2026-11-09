use std::process::Command;

#[test]
fn helper_resolves_names_without_treating_numeric_names_as_uids()
-> Result<(), Box<dyn std::error::Error>> {
    let executable = env!("CARGO_BIN_EXE_ruxel-agent");
    for name in ["root", "0", "4294967294"] {
        let expected = nix::unistd::User::from_name(name)?;
        let output = Command::new(executable)
            .args(["--ruxel-nss-helper", name])
            .env_clear()
            .output()?;
        match expected.filter(|account| account.name == name) {
            Some(account) => {
                assert!(output.status.success());
                let record = String::from_utf8(output.stdout)?;
                let fields: Vec<_> = record.trim_end_matches('\n').split(':').collect();
                assert_eq!(fields[0], name);
                assert_eq!(fields[2], account.uid.as_raw().to_string());
                assert_eq!(fields[3], account.gid.as_raw().to_string());
            }
            None => assert!(!output.status.success()),
        }
    }
    Ok(())
}
