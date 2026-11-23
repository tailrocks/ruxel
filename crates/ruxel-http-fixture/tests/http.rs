#![forbid(unsafe_code)]
use ruxel_http_fixture::{MAX_HEADER, bind, respond};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
    thread,
    time::Duration,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "ruxel-http-test-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn request(root: PathBuf, bytes: Vec<u8>) -> Vec<u8> {
    let listener = bind(0).unwrap();
    let address = listener.local_addr().unwrap();
    assert!(address.ip().is_loopback());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        respond(&mut stream, &root).unwrap();
    });
    let mut client = TcpStream::connect(address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    client.write_all(&bytes).unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    server.join().unwrap();
    response
}
#[test]
fn exact_binary_get_head_and_dynamic_update() {
    let f = Fixture::new();
    let payload = b"synthetic-download\0\xff\r\n";
    fs::write(f.0.join("payload-source.txt"), payload).unwrap();
    for method in ["GET", "HEAD"] {
        let response = request(
            f.0.clone(),
            format!("{method} /payload-source.txt HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .into_bytes(),
        );
        let split = response.windows(4).position(|s| s == b"\r\n\r\n").unwrap() + 4;
        let header = std::str::from_utf8(&response[..split]).unwrap();
        assert!(header.starts_with("HTTP/1.0 200 OK\r\n"));
        assert!(header.contains(&format!("Content-Length: {}\r\n", payload.len())));
        assert_eq!(
            &response[split..],
            if method == "GET" {
                payload.as_slice()
            } else {
                b""
            }
        );
    }
    fs::write(f.0.join("payload-source.txt"), "changed").unwrap();
    assert!(
        request(
            f.0.clone(),
            b"GET /payload-source.txt?query HTTP/1.0\r\n\r\n".to_vec()
        )
        .ends_with(b"changed")
    );
}
#[test]
fn rejects_other_paths_methods_and_oversized_headers() {
    let f = Fixture::new();
    fs::write(f.0.join("payload-source.txt"), "synthetic").unwrap();
    for (path, status) in [
        ("/", "404"),
        ("/../payload-source.txt", "404"),
        ("/%2e%2e/payload-source.txt", "404"),
    ] {
        let response = request(
            f.0.clone(),
            format!("GET {path} HTTP/1.0\r\n\r\n").into_bytes(),
        );
        assert!(
            String::from_utf8(response)
                .unwrap()
                .starts_with(&format!("HTTP/1.0 {status}"))
        );
    }
    let response = request(
        f.0.clone(),
        b"POST /payload-source.txt HTTP/1.0\r\n\r\n".to_vec(),
    );
    assert!(response.starts_with(b"HTTP/1.0 405"));
    let response = request(f.0.clone(), vec![b'x'; MAX_HEADER]);
    assert!(response.starts_with(b"HTTP/1.0 431"));
}
#[test]
fn empty_and_missing_payload() {
    let f = Fixture::new();
    let req = b"GET /payload-source.txt HTTP/1.0\r\n\r\n".to_vec();
    assert!(request(f.0.clone(), req.clone()).starts_with(b"HTTP/1.0 404"));
    fs::write(f.0.join("payload-source.txt"), []).unwrap();
    let response = request(f.0.clone(), req);
    assert!(response.starts_with(b"HTTP/1.0 200"));
    assert!(response.ends_with(b"Content-Length: 0\r\nConnection: close\r\n\r\n"));
}
#[test]
fn oversized_payload_and_outside_symlink_are_refused() {
    let f = Fixture::new();
    let payload = f.0.join("payload-source.txt");
    fs::write(
        &payload,
        vec![b'x'; (ruxel_http_fixture::MAX_PAYLOAD + 1) as usize],
    )
    .unwrap();
    let req = b"GET /payload-source.txt HTTP/1.0\r\n\r\n".to_vec();
    assert!(request(f.0.clone(), req.clone()).starts_with(b"HTTP/1.0 404"));
    fs::remove_file(&payload).unwrap();
    let outside = f.0.with_extension("outside");
    fs::write(&outside, "outside-synthetic").unwrap();
    std::os::unix::fs::symlink(&outside, &payload).unwrap();
    let response = request(f.0.clone(), req);
    fs::remove_file(outside).unwrap();
    assert!(response.starts_with(b"HTTP/1.0 404"));
    assert!(!response.windows(17).any(|w| w == b"outside-synthetic"));
}
