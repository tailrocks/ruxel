use std::io::{Read, Write};

fn first_connection() -> Result<bool, Box<dyn std::error::Error>> {
    let state =
        std::path::PathBuf::from(std::env::var_os("RUXEL_STATE_DIR").ok_or("missing state")?);
    let parent = state.parent().ok_or("missing state parent")?;
    std::fs::create_dir_all(parent)?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(parent.join("first-connection"))
    {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn failed_task(output: &mut impl Write) -> std::io::Result<()> {
    // The fixture has one agent task (ID 1), with a valid failed TaskResult.
    output.write_all(b"\x16\x22\x14\x08\x01\x12\x06failed\x22\x08{\"rc\":1}")?;
    output.flush()
}

fn frame(input: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut length = 0usize;
    for shift in (0..=28).step_by(7) {
        let mut byte = [0];
        input.read_exact(&mut byte)?;
        length |= usize::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 && length <= 1024 {
            let mut body = vec![0; length];
            input.read_exact(&mut body)?;
            return Ok(body);
        }
    }
    Err(std::io::Error::other("invalid fixture frame length"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("RUXEL_TEST_DELEGATE_FAILURE").is_some() && !first_connection()? {
        return Err("synthetic delegated connection failure".into());
    }
    let mut input = std::io::stdin().lock();
    let hello = frame(&mut input)?;
    if hello.len() < 4 || hello[0] != 10 || hello[2] != 8 || hello[3] >= 128 {
        return Err("expected fixture Hello with a one-byte protocol version".into());
    }
    // Event.HelloAck: echo the protocol version and explicitly report no sensitive effects.
    let mut output = std::io::stdout().lock();
    output.write_all(&[6, 10, 4, 16, hello[3], 40, 0])?;
    output.flush()?;
    let mut next = frame(&mut input)?;
    if next.first() == Some(&18) {
        match std::env::var("RUXEL_TEST_TASK_RESPONSE").as_deref() {
            Ok("disconnect") => return Ok(()),
            Ok("failed") => failed_task(&mut output)?,
            _ => return Err("unexpected fixture Plan".into()),
        }
        next = frame(&mut input)?;
    }
    if next != [42, 0] {
        return Err("expected Done".into());
    }
    if std::env::var_os("RUXEL_TEST_SHUTDOWN_FAILURE").is_some() {
        std::process::exit(19);
    }
    Ok(())
}
