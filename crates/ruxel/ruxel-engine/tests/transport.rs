use ruxel_engine::transport::ConnectOptions;

#[test]
fn defaults_require_existing_host_key() {
    let options = ConnectOptions::default();
    assert!(options.keyfile.is_none());
    assert!(options.known_hosts_file.is_none());
    assert!(!options.accept_new_host_key);
    assert!(!options.diff_mode);
    assert!(!options.no_cache);
}
