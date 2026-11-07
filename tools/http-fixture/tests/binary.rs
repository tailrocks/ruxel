#![forbid(unsafe_code)]
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ruxel-http-fixture"))
}
fn bounded_status(mut command: Command) -> std::process::ExitStatus {
    let mut child = Guard(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            return status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "fixture startup did not terminate"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn startup_validation_and_version() {
    let mut c = command();
    c.arg("--version");
    assert!(bounded_status(c).success());
    for args in [
        vec!["--directory", "/nonexistent-ruxel-synthetic-http-root"],
        vec!["--directory", ".", "--port", "not-a-port"],
        vec!["--bind", "0.0.0.0"],
    ] {
        let mut c = command();
        c.args(args);
        assert!(!bounded_status(c).success());
    }
}
#[test]
fn actual_binary_readiness_get_and_shutdown() {
    let root = std::env::temp_dir().join(format!("ruxel-http-binary-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("payload-source.txt"), "synthetic-download").unwrap();
    let mut child = Guard(
        command()
            .args(["--directory", root.to_str().unwrap(), "--port", "0"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = tx.send(result);
    });
    let ready = rx.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    let (endpoint, pid) = ready.trim().split_once(" pid=").unwrap();
    assert_eq!(pid.parse::<u32>().unwrap(), child.0.id());
    let address = endpoint
        .trim()
        .strip_prefix("listening ")
        .unwrap()
        .parse::<std::net::SocketAddr>()
        .unwrap();
    assert!(address.ip().is_loopback());
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(b"GET /payload-source.txt HTTP/1.0\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.0 200 OK\r\n"));
    assert!(response.ends_with("synthetic-download"));
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    drop(child);
    assert!(TcpStream::connect(address).is_err());
    fs::remove_dir_all(root).unwrap();
}
