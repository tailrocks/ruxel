use super::{File, Root, context, fs};
use crate::modules::get_url::{client, fetch_with_programs};
use std::os::unix::fs::PermissionsExt;

fn client_script(root: &Root) -> std::path::PathBuf {
    let client = root.0.join("client");
    fs::write(
        &client,
        "#!/bin/sh\nprintf '%s\\n' \"$https_proxy\" \"$PATH\" \"${LD_PRELOAD-unset}\" \"${DYLD_INSERT_LIBRARIES-unset}\" \"${WGETRC-unset}\" \"${CURL_HOME-unset}\" \"${RUXEL_DOWNLOAD_TEST-unset}\" \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&client, fs::Permissions::from_mode(0o700)).unwrap();
    client
}

#[test]
fn clients_keep_proxy_but_drop_task_path_loader_and_config_environment() {
    let root = Root::new();
    let client = client_script(&root);
    let mut ctx = context(false);
    ctx.environment = [
        ("https_proxy", "http://proxy.example.invalid:3128"),
        ("PATH", "/untrusted/task/bin"),
        ("LD_PRELOAD", "/untrusted/loader.so"),
        ("DYLD_INSERT_LIBRARIES", "/untrusted/loader.dylib"),
        ("WGETRC", "/untrusted/wgetrc"),
        ("CURL_HOME", "/untrusted/curl"),
        ("RUXEL_DOWNLOAD_TEST", "untrusted"),
    ]
    .map(|(name, value)| (name.into(), value.into()))
    .into();
    for (name, curl, wget, options) in [
        ("curl", Some(client.as_path()), None, "--disable\n-fsSL"),
        (
            "wget",
            None,
            Some(client.as_path()),
            "--no-config\n-q\n-O\n-",
        ),
    ] {
        let output = root.0.join(name);
        let file = File::create(&output).unwrap();
        assert!(
            fetch_with_programs("https://example.invalid/file", &file, curl, wget, &ctx).unwrap()
        );
        assert_eq!(
            fs::read_to_string(output).unwrap(),
            format!(
                "http://proxy.example.invalid:3128\n/usr/bin:/bin\nunset\nunset\nunset\nunset\nunset\n{options}\n--\nhttps://example.invalid/file\n"
            )
        );
    }
}

#[test]
fn trusted_absolute_client_resolution_ignores_task_path() {
    let root = Root::new();
    let malicious = root.0.join("curl");
    fs::write(&malicious, "#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(&malicious, fs::Permissions::from_mode(0o700)).unwrap();
    let mut ctx = context(false);
    ctx.environment = vec![("PATH".into(), root.0.to_string_lossy().into_owned())];
    for name in ["curl", "wget"] {
        if let Some(program) = client::resolve(name).unwrap() {
            assert!(program.is_absolute());
            assert_ne!(program, malicious);
            assert_eq!(client::command(&program, &ctx).get_program(), program);
        }
    }
    assert_eq!(
        fs::read_to_string(malicious).unwrap(),
        "#!/bin/sh\nexit 99\n"
    );
}

#[test]
fn unavailable_clients_and_client_failures_do_not_launch_task_executables() {
    let root = Root::new();
    let output = File::create(root.0.join("output")).unwrap();
    assert!(
        !fetch_with_programs(
            "https://example.invalid/file",
            &output,
            None,
            None,
            &context(false)
        )
        .unwrap()
    );
    let failing = root.0.join("failing");
    fs::write(&failing, "#!/bin/sh\nexit 42\n").unwrap();
    fs::set_permissions(&failing, fs::Permissions::from_mode(0o700)).unwrap();
    let fallback = client_script(&root);
    assert!(
        fetch_with_programs(
            "https://example.invalid/file",
            &output,
            Some(&failing),
            Some(&fallback),
            &context(false)
        )
        .unwrap_err()
        .starts_with("curl failed:")
    );
    assert_eq!(fs::metadata(root.0.join("output")).unwrap().len(), 0);
}

#[test]
fn inherited_environment_is_cleared_before_proxy_filtering() {
    const CHILD: &str = "RUXEL_DOWNLOAD_ENV_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let root = Root::new();
        let client = client_script(&root);
        let output = client::command(&client, &context(false)).output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.starts_with("http://proxy.example.invalid:3128\n/usr/bin:/bin\n"));
        assert!(!text.contains("untrusted"));
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "modules::get_url::download_tests::clients::inherited_environment_is_cleared_before_proxy_filtering"])
        .env(CHILD, "1")
        .env("https_proxy", "http://proxy.example.invalid:3128")
        .env("PATH", "/untrusted/bin")
        .env("RUXEL_DOWNLOAD_TEST", "untrusted")
        .env("CURL_HOME", "/untrusted/curl")
        .env("WGETRC", "/untrusted/wgetrc")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[test]
fn valid_curl_never_resolves_an_untrusted_unused_wget() {
    let root = Root::new();
    let client = client_script(&root);
    let output = File::create(root.0.join("output")).unwrap();
    let result = crate::modules::get_url::fetch_with_resolver(
        "https://example.invalid/file",
        &output,
        &context(false),
        |name| match name {
            "curl" => Ok(Some(client.clone())),
            _ => Err("unused wget is untrusted".into()),
        },
    );
    assert!(result.unwrap());
}
