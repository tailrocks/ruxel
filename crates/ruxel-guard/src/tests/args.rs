use super::*;

#[test]
fn argument_vectors_are_complete() {
    let common = [
        "-F",
        "none",
        "-o",
        "HostName=192.0.2.10",
        "-o",
        "User=test",
        "-o",
        "Port=22",
        "-o",
        "IdentitiesOnly=yes",
        "-o",
        "IdentityAgent=none",
        "-o",
    ];
    for (mode, checking, control, timeout) in [
        (
            ArgsMode::Normal,
            "StrictHostKeyChecking=yes",
            "ControlPath=\"/tmp/mux\"",
            false,
        ),
        (
            ArgsMode::Fresh,
            "StrictHostKeyChecking=yes",
            "ControlPath=none",
            true,
        ),
        (
            ArgsMode::Trust,
            "StrictHostKeyChecking=accept-new",
            "ControlPath=none",
            true,
        ),
    ] {
        let mut expected = common.to_vec();
        expected.extend([
            checking,
            "-o",
            "GlobalKnownHostsFile=none",
            "-o",
            "ForwardAgent=no",
            "-o",
            "ProxyJump=none",
            "-o",
            "ProxyCommand=none",
            "-o",
            "BatchMode=yes",
            "-o",
            "IdentityFile=\"/tmp/key\"",
            "-o",
            "UserKnownHostsFile=\"/tmp/known\"",
            "-o",
            control,
        ]);
        if timeout {
            expected.extend(["-o", "ConnectTimeout=5"]);
        }
        expected.push("192.0.2.10");
        let actual = resolve::build_args(&spec(), &PathBuf::from("/tmp/mux"), mode);
        assert_eq!(actual, expected);
    }
}
