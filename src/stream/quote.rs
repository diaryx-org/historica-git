//! The path quoting the stream uses, both ways.
//!
//! A path in a fast-import stream is either raw bytes or a C-style quoted
//! string, and which one it is is decided by the first byte: a `"` opens a
//! quoted string and nothing else does. Git quotes when it has to — a path
//! holding a quote, a backslash, a newline, or a byte outside printable ASCII —
//! and, as `git fast-export` is observed to do, when the path holds a space.
//!
//! Both directions live here because they are one rule read twice, and a rule
//! implemented twice drifts.

use std::fmt;

/// What went wrong reading a quoted path. The caller adds the line number; this
/// says what the bytes did.
#[derive(Debug, PartialEq, Eq)]
pub enum Unquoted {
    /// The closing quote never arrived.
    Unterminated,
    /// A `\` at the end of the string, escaping nothing.
    DanglingEscape,
    /// A `\` followed by something that is not an escape this format has.
    UnknownEscape(u8),
}

impl fmt::Display for Unquoted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unquoted::Unterminated => write!(f, "quoted path has no closing `\"`"),
            Unquoted::DanglingEscape => {
                write!(f, "quoted path ends in a `\\` that escapes nothing")
            }
            Unquoted::UnknownEscape(byte) => write!(
                f,
                "`\\{}` is not an escape this format has — the escapes are \
                 \\a \\b \\f \\n \\r \\t \\v \\\\ \\\" and \\ooo",
                *byte as char
            ),
        }
    }
}

/// Read a quoted path, and say where it ended.
///
/// `input` must begin at the opening `"`. Returns the path's real bytes and the
/// offset just past the closing quote, so a caller reading `R <src> <dst>` can
/// carry on at the space.
pub fn unquote(input: &[u8]) -> Result<(Vec<u8>, usize), Unquoted> {
    debug_assert_eq!(input.first(), Some(&b'"'));
    let mut out = Vec::new();
    let mut at = 1;
    while at < input.len() {
        match input[at] {
            b'"' => return Ok((out, at + 1)),
            b'\\' => {
                let escape = *input.get(at + 1).ok_or(Unquoted::DanglingEscape)?;
                at += 2;
                out.push(match escape {
                    b'a' => 0x07,
                    b'b' => 0x08,
                    b'f' => 0x0c,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'v' => 0x0b,
                    b'\\' => b'\\',
                    b'"' => b'"',
                    // `\ooo`, one to three octal digits. Git writes three and
                    // reads up to three; reading fewer costs nothing and
                    // accepts a stream some other frontend wrote.
                    b'0'..=b'7' => {
                        let mut value = u32::from(escape - b'0');
                        let mut digits = 1;
                        while digits < 3 {
                            match input.get(at) {
                                Some(next @ b'0'..=b'7') => {
                                    value = value * 8 + u32::from(next - b'0');
                                    at += 1;
                                    digits += 1;
                                }
                                _ => break,
                            }
                        }
                        // Three octal digits can spell 0o777, which is larger
                        // than a byte. Git never writes one; refuse rather than
                        // truncate, because truncating invents a path.
                        u8::try_from(value).map_err(|_| Unquoted::UnknownEscape(escape))?
                    }
                    other => return Err(Unquoted::UnknownEscape(other)),
                });
            }
            byte => {
                out.push(byte);
                at += 1;
            }
        }
    }
    Err(Unquoted::Unterminated)
}

/// Write a path the way the stream wants it: raw if it can be, quoted if it
/// must be.
///
/// `must_quote_space` is for the one position where a space is not allowed to
/// be raw — the source path of a `filerename` or `filecopy`, which the reader
/// ends at the first space because it has to find the destination.
pub fn quote(path: &[u8], must_quote_space: bool) -> Vec<u8> {
    let plain = |byte: u8| match byte {
        b'"' | b'\\' => false,
        b' ' => !must_quote_space,
        // Printable ASCII above the space, which is what may travel raw.
        // Anything else is quoted so that a stream stays byte-exact through
        // tools that would otherwise be tempted to re-encode it.
        0x21..=0x7e => true,
        _ => false,
    };
    if !path.is_empty() && path.iter().copied().all(plain) {
        return path.to_vec();
    }
    let mut out = vec![b'"'];
    for &byte in path {
        match byte {
            0x07 => out.extend_from_slice(b"\\a"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0c => out.extend_from_slice(b"\\f"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x0b => out.extend_from_slice(b"\\v"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'"' => out.extend_from_slice(b"\\\""),
            0x20..=0x7e => out.push(byte),
            other => out.extend_from_slice(format!("\\{other:03o}").as_bytes()),
        }
    }
    out.push(b'"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quoted_path_ends_at_its_closing_quote() {
        let (path, past) = unquote(br#""sub/with space.txt" and more"#).unwrap();
        assert_eq!(path, b"sub/with space.txt");
        assert_eq!(past, 20);
    }

    #[test]
    fn every_escape_reads() {
        let (path, _) = unquote(br#""\a\b\f\n\r\t\v\\\"\303\251""#).unwrap();
        assert_eq!(path, b"\x07\x08\x0c\n\r\t\x0b\\\"\xc3\xa9");
    }

    #[test]
    fn a_short_octal_escape_reads() {
        assert_eq!(unquote(br#""\0""#).unwrap().0, b"\0");
        assert_eq!(unquote(br#""\12""#).unwrap().0, b"\n");
    }

    #[test]
    fn what_is_refused_says_why() {
        assert_eq!(unquote(br#""unclosed"#), Err(Unquoted::Unterminated));
        assert_eq!(unquote(br#""trailing\"#), Err(Unquoted::DanglingEscape));
        assert_eq!(unquote(br#""\q""#), Err(Unquoted::UnknownEscape(b'q')));
        // 0o777 does not fit in a byte.
        assert_eq!(unquote(br#""\777""#), Err(Unquoted::UnknownEscape(b'7')));
    }

    #[test]
    fn a_plain_path_travels_raw() {
        assert_eq!(quote(b"docs/README.md", false), b"docs/README.md");
        assert_eq!(quote(b"with space.txt", false), b"with space.txt");
        assert_eq!(quote(b"with space.txt", true), br#""with space.txt""#);
    }

    #[test]
    fn quoting_and_unquoting_are_one_rule() {
        for path in [
            &b"plain.txt"[..],
            b"with space.txt",
            b"quote\".txt",
            b"back\\slash",
            b"new\nline",
            b"caf\xc3\xa9",
            b"\x00\x01\x02",
        ] {
            for must_quote_space in [false, true] {
                let written = quote(path, must_quote_space);
                let read = match written.first() {
                    Some(b'"') => unquote(&written).unwrap().0,
                    _ => written.clone(),
                };
                assert_eq!(read, path, "round trip failed for {path:?}");
            }
        }
    }
}
