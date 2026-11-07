use anyhow::{Result, bail, ensure};
fn raw_end(bytes: &[u8], start: usize) -> Result<Option<usize>> {
    if bytes.get(start) != Some(&b'r') {
        return Ok(None);
    }
    let mut quote = start + 1;
    while bytes.get(quote) == Some(&b'#') {
        quote += 1;
    }
    if bytes.get(quote) != Some(&b'"') || quote - start - 1 > 255 {
        return Ok(None);
    }
    let hashes = quote - start - 1;
    let mut i = quote + 1;
    while i < bytes.len() {
        if bytes[i] == b'"'
            && bytes
                .get(i + 1..i + 1 + hashes)
                .is_some_and(|s| s.iter().all(|c| *c == b'#'))
        {
            return Ok(Some(i + 1 + hashes));
        }
        i += 1;
    }
    bail!("unterminated Rust raw string")
}
fn string_end(bytes: &[u8], start: usize) -> Result<usize> {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Ok(i + 1),
            _ => i += 1,
        }
    }
    bail!("unterminated Rust string")
}
pub fn strip(source: &str) -> Result<String> {
    let b = source.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b.get(i..i + 2) == Some(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b.get(i..i + 2) == Some(b"/*") {
            i += 2;
            let mut depth = 1;
            while i < b.len() && depth > 0 {
                if b.get(i..i + 2) == Some(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b.get(i..i + 2) == Some(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
                    i += 1;
                }
            }
            ensure!(depth == 0, "unterminated Rust block comment");
        } else if b[i] == b'"' {
            let end = string_end(b, i)?;
            out.extend_from_slice(&b[i..end]);
            i = end;
        } else if let Some(end) = raw_end(b, i)? {
            out.extend_from_slice(&b[i..end]);
            i = end;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Ok(String::from_utf8(out)?)
}
pub fn mask(source: &str) -> Result<String> {
    let b = source.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = raw_end(b, i)? {
            for byte in &mut out[i..end] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
            i = end;
        } else if b[i] == b'"' {
            let end = string_end(b, i)?;
            for byte in &mut out[i + 1..end - 1] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    Ok(String::from_utf8(out)?)
}

pub fn block(source: &str, open: usize) -> Result<std::ops::Range<usize>> {
    let bytes = source.as_bytes();
    ensure!(bytes.get(open) == Some(&b'{'), "missing opening brace");
    let mut depth = 1_usize;
    let mut i = open + 1;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            // Character literals may contain braces; lifetimes have no closing quote here.
            if bytes.get(i + 1) == Some(&b'\\') {
                if let Some(end) = bytes[i + 2..].iter().position(|b| *b == b'\'') {
                    let candidate = &bytes[i + 2..i + 2 + end];
                    if !candidate.contains(&b'\n') {
                        i += end + 3;
                        continue;
                    }
                }
            } else if bytes.get(i + 2) == Some(&b'\'') {
                i += 3;
                continue;
            }
        }
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(open + 1..i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    bail!("unterminated code block")
}

pub fn function(source: &str, name: &str) -> Result<std::ops::Range<usize>> {
    function_with_prefix(source, name, r"async\s+")
}

pub fn sync_function(source: &str, name: &str) -> Result<std::ops::Range<usize>> {
    function_with_prefix(source, name, "")
}

fn function_with_prefix(source: &str, name: &str, prefix: &str) -> Result<std::ops::Range<usize>> {
    let pattern = format!(
        r"(?s)\b{}fn\s+{}\s*\([^)]*\)\s*(?:->\s*[^{{]+)?\{{",
        prefix,
        regex::escape(name)
    );
    let re = regex::Regex::new(&pattern)?;
    let matches: Vec<_> = re.find_iter(source).collect();
    ensure!(matches.len() == 1, "missing or ambiguous function {name}");
    block(source, matches[0].end() - 1)
}
