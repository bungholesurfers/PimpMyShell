//! Wire protocol between the `pms` CLI and the TAP inside explorer.
//!
//! One duplex byte pipe; each request and each response is a frame: u32 LE length + UTF-8.
//! A request is a command line; arguments are whitespace-separated, `"double quotes"` group.

use std::io::{self, Read, Write};

pub const PIPE_NAME: &str = r"\\.\pipe\pms-tap";

pub fn write_frame(w: &mut impl Write, s: &str) -> io::Result<()> {
    w.write_all(&(s.len() as u32).to_le_bytes())?;
    w.write_all(s.as_bytes())?;
    w.flush()
}

/// `Ok(None)` on clean EOF (peer hung up between frames).
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<String>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Err(e) if matches!(e.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe) => return Ok(None),
        r => r?,
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > 64 << 20 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else { break };
        let mut tok = String::new();
        if first == '"' {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' if matches!(chars.peek(), Some('"' | '\\')) => tok.push(chars.next().unwrap()),
                    c => tok.push(c),
                }
            }
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                tok.push(c);
            }
        }
        out.push(tok);
    }
    out
}

pub fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains(|c: char| c.is_whitespace() || c == '"') {
        return arg.to_string();
    }
    let mut s = String::from('"');
    for c in arg.chars() {
        if matches!(c, '"' | '\\') {
            s.push('\\');
        }
        s.push(c);
    }
    s.push('"');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let args = ["set", "BackgroundFill/Fill", "TintColor", "", "a \"b\" \\c"];
        let line = args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
        assert_eq!(tokenize(&line), args);
    }
}
