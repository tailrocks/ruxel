#![forbid(unsafe_code)]
use ruxel_http_fixture::{bind, respond};
use std::{
    io::Write,
    net::TcpStream,
    path::Path,
    thread,
    time::{Duration, Instant},
};
#[test]
fn progressing_header_cannot_extend_absolute_deadline() {
    let listener = bind(0).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let start = Instant::now();
        let error = respond(&mut stream, Path::new(".")).unwrap_err();
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        ));
        assert!(start.elapsed() < Duration::from_secs(4));
    });
    let mut client = TcpStream::connect(address).unwrap();
    let writer = thread::spawn(move || {
        for _ in 0..7 {
            if client.write_all(b"x").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(400));
        }
    });
    server.join().unwrap();
    writer.join().unwrap();
}
