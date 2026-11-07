#![forbid(unsafe_code)]
use ruxel_http_fixture::{bind, respond};
use std::{
    io::{self, Write},
    path::PathBuf,
};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(std::ffi::OsStr::new("--version")) && args.next().is_none() {
        println!("ruxel-http-fixture {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let mut args = first.into_iter().chain(args);
    let (mut directory, mut port) = (None, 18080u16);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--directory") => {
                directory = Some(PathBuf::from(args.next().ok_or("missing directory")?))
            }
            Some("--port") => {
                port = args
                    .next()
                    .and_then(|p| p.into_string().ok())
                    .ok_or("missing port")?
                    .parse()?
            }
            _ => return Err("usage: ruxel-http-fixture --directory PATH [--port PORT]".into()),
        }
    }
    let root = directory.ok_or("missing --directory")?.canonicalize()?;
    if !root.is_dir() {
        return Err("fixture directory is not a directory".into());
    }
    let listener = bind(port)?;
    println!(
        "listening {} pid={}",
        listener.local_addr()?,
        std::process::id()
    );
    io::stdout().flush()?;
    for connection in listener.incoming() {
        let mut connection = connection?;
        if let Err(error) = respond(&mut connection, &root) {
            eprintln!("fixture request failed: {}", error.kind());
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("http-fixture: {error}");
        std::process::exit(1);
    }
}
