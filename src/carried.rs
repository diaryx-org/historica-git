//! The git facts historica has no word for, carried across as headers.
//!
//! Decision 0004 makes a commit a function of the revision it came from, and
//! measured the set that does not hold: a committer distinct from the author,
//! and a signature. Both are in the commit's own bytes, so a revision that
//! cannot state them cannot be written back as the commit it came from.
//!
//! Historica's decision 0065 has the door and its 0070 opened it: a key with a
//! dot in it is another tool's vocabulary, parsed, hashed with everything else,
//! never interpreted, and carried across an amendment. So the fact lives in the
//! revision's own bytes, the revision's identity covers it, and a rewrite does
//! not silently drop what made the commit what it was.
//!
//! The keys are `git.`-qualified for the tool that owns them, which is this
//! one.

use crate::stream::{Person, quote, unquote};

/// The committer, where the commit named one other than its author.
///
/// The whole line git writes, since every part of it is in the commit's bytes:
/// `Name <email> 1767348245 -0700`.
pub const COMMITTER: &str = "git.committer";

/// The signature, exactly as the stream carried it.
pub const SIGNATURE: &str = "git.signature";

/// What `gpgsig` said the signature was: `sha1 openpgp`.
///
/// Its own key rather than a prefix on the signature, because it is a different
/// fact — what the bytes are — and joining two facts in one value would need a
/// grammar to take them apart again.
pub const SIGNATURE_KIND: &str = "git.signature-kind";

/// Bytes as a header value: raw where they can be read, quoted where they
/// cannot.
///
/// Historica's rule for any header value is one line, no control characters,
/// and no leading or trailing space. A committer satisfies it as it stands and
/// stays readable; a signature is an armoured block with newlines in it and
/// does not. So the one escaping rule this crate already has — the stream's
/// own, in [`crate::stream::quote`] — does the second case, which means the
/// escape a person meets in a revision document is the escape they already met
/// in the stream.
pub fn spell(bytes: &[u8]) -> String {
    let plain = quote(bytes, false);
    // `quote` lets a space travel raw, and a value historica takes may not
    // begin or end with one. Quoting spaces too is the fallback, and it is only
    // ever reached by a name somebody padded.
    let spelled = match plain.first() == Some(&b' ') || plain.last() == Some(&b' ') {
        true => quote(bytes, true),
        false => plain,
    };
    String::from_utf8_lossy(&spelled).into_owned()
}

/// The bytes a header value spells, whichever way [`spell`] wrote it.
pub fn read(value: &str) -> Option<Vec<u8>> {
    match value.as_bytes().first() {
        Some(b'"') => unquote(value.as_bytes()).ok().map(|(bytes, _)| bytes),
        _ => Some(value.as_bytes().to_vec()),
    }
}

/// A person and a time, as the one line git writes them on.
pub fn spell_person(person: &Person) -> String {
    let mut line = person.name.clone();
    line.extend_from_slice(b" <");
    line.extend_from_slice(&person.email);
    line.extend_from_slice(b"> ");
    line.extend_from_slice(person.seconds.to_string().as_bytes());
    line.push(b' ');
    line.extend_from_slice(offset(person.offset_minutes).as_bytes());
    spell(&line)
}

/// The same line, read back.
///
/// `None` where the value is not one, which is a header somebody else wrote
/// under a key this tool owns — reported rather than guessed at.
pub fn read_person(value: &str) -> Option<Person> {
    let text = read(value)?;
    let close = text.iter().rposition(|&byte| byte == b'>')?;
    let open = text[..close].iter().rposition(|&byte| byte == b'<')?;
    let name = text[..open]
        .iter()
        .rev()
        .skip_while(|byte| **byte == b' ')
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<u8>>();
    let email = text[open + 1..close].to_vec();

    let when = String::from_utf8_lossy(text.get(close + 1..)?).into_owned();
    let when = when.trim();
    let (seconds, offset) = when.split_once(' ')?;
    let seconds: i64 = seconds.trim().parse().ok()?;

    let offset = offset.trim();
    if offset.len() != 5 {
        return None;
    }
    let sign = match offset.as_bytes()[0] {
        b'-' => -1,
        b'+' => 1,
        _ => return None,
    };
    let hours: i32 = offset[1..3].parse().ok()?;
    let minutes: i32 = offset[3..5].parse().ok()?;

    Some(Person {
        name,
        email,
        seconds,
        offset_minutes: sign * (hours * 60 + minutes),
    })
}

/// Minutes east of UTC, as git's `+HHMM`.
pub fn offset(minutes: i32) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let minutes = minutes.abs();
    format!("{sign}{:02}{:02}", minutes / 60, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person() -> Person {
        Person {
            name: b"Ada Lovelace".to_vec(),
            email: b"ada@example.com".to_vec(),
            seconds: 1767348245,
            offset_minutes: -420,
        }
    }

    #[test]
    fn a_committer_stays_readable() {
        assert_eq!(
            spell_person(&person()),
            "Ada Lovelace <ada@example.com> 1767348245 -0700"
        );
        assert_eq!(read_person(&spell_person(&person())), Some(person()));
    }

    #[test]
    fn a_signature_survives_its_newlines() {
        let armour = b"-----BEGIN PGP SIGNATURE-----\n\nabc/+=\n-----END PGP SIGNATURE-----";
        let spelled = spell(armour);
        assert!(!spelled.contains('\n'), "a header is one line: {spelled}");
        assert!(
            spelled.starts_with("\"-----BEGIN PGP SIGNATURE-----"),
            "what it is should still be legible: {spelled}"
        );
        assert_eq!(read(&spelled).as_deref(), Some(&armour[..]));
    }

    /// What historica will accept, checked here rather than discovered by a
    /// `record` that refuses halfway through a conversion.
    #[test]
    fn what_is_spelled_is_a_header_historica_takes() {
        let armour = b"-----BEGIN PGP SIGNATURE-----\n\nabc\n-----END-----";
        for (key, value) in [
            (COMMITTER, spell_person(&person())),
            (SIGNATURE, spell(armour)),
            (SIGNATURE_KIND, spell(b"sha1 openpgp")),
            // A name somebody padded, which is the case `spell` falls back for.
            (COMMITTER, spell(b"  padded  ")),
        ] {
            historica::format::check_extension(key, &value)
                .unwrap_or_else(|because| panic!("`{key}` = `{value}` refused: {because:?}"));
        }
    }
}
