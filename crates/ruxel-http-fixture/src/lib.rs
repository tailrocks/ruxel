#![forbid(unsafe_code)]
use std::{
    fs::File,
    io::{self, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::Path,
    time::{Duration, Instant},
};
pub const MAX_HEADER: usize = 8192;
pub const MAX_PAYLOAD: u64 = 8 * 1024 * 1024;
pub fn bind(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
}
fn header(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    while bytes.len() < MAX_HEADER {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "request header deadline",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return Ok(bytes);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "request header too large",
    ))
}
fn reply(stream: &mut TcpStream, status: &str, body: &[u8], head: bool) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.0 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if !head {
        stream.write_all(body)?;
    }
    stream.flush()
}
fn payload(root: &Path) -> io::Result<Vec<u8>> {
    let root = root.canonicalize()?;
    let path = root.join("payload-source.txt").canonicalize()?;
    if !path.starts_with(&root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "payload outside fixture directory",
        ));
    }
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "payload is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PAYLOAD + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "payload too large",
        ));
    }
    Ok(bytes)
}
pub fn respond(stream: &mut TcpStream, root: &Path) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let bytes = match header(stream) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return reply(
                stream,
                "431 Request Header Fields Too Large",
                b"header too large",
                false,
            );
        }
        Err(error) => return Err(error),
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(_) => return reply(stream, "400 Bad Request", b"invalid request", false),
    };
    let words: Vec<_> = text
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    if words.len() != 3 || !matches!(words[2], "HTTP/1.0" | "HTTP/1.1") {
        return reply(stream, "400 Bad Request", b"invalid request", false);
    }
    let head = words[0] == "HEAD";
    if !matches!(words[0], "GET" | "HEAD") {
        return reply(
            stream,
            "405 Method Not Allowed",
            b"unsupported method",
            false,
        );
    }
    if words[1].split('?').next() != Some("/payload-source.txt") {
        return reply(stream, "404 Not Found", b"missing payload", head);
    }
    match payload(root) {
        Ok(body) => reply(stream, "200 OK", &body, head),
        Err(_) => reply(stream, "404 Not Found", b"missing payload", head),
    }
}
