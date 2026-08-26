//! Reading the stream.
//!
//! Strict, in the way historica's own parsers are strict: a stream this accepts
//! is one git wrote, and anything else is refused with the line it went wrong
//! on and what would have been right. A converter that guessed at a line it did
//! not understand would produce a history nobody asked for, and the person
//! holding it would have no way to tell.
//!
//! The reader yields one command at a time from anything that implements
//! [`BufRead`], so a repository's whole export never has to be held at once —
//! only the blob currently being read.

use std::error;
use std::fmt;
use std::io::{self, BufRead};

use super::quote::unquote;
use super::{
    Blob, Change, Command, Commit, Content, DataRef, Mark, Mode, Person, Reset, Signature, Tag,
};

/// A stream this reader would not read, or an input it could not read from.
#[derive(Debug)]
pub struct Error {
    /// The line the trouble is on, counting from one.
    pub line: usize,
    message: String,
    source: Option<io::Error>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e as &(dyn error::Error + 'static))
    }
}

/// Reads a fast-import stream, one command at a time.
pub struct Reader<R> {
    input: R,
    line: usize,
    /// One line of lookahead. A commit ends where its change lines stop, which
    /// is only knowable by reading the line that is not one.
    pending: Option<Vec<u8>>,
}

impl<R: BufRead> Reader<R> {
    /// Read a stream from anything buffered — a file, a pipe from `git
    /// fast-export`, or bytes in memory.
    pub fn new(input: R) -> Self {
        Reader {
            input,
            line: 0,
            pending: None,
        }
    }

    /// The next command, or `None` at the end of the stream.
    ///
    /// [`Reader`] is also an [`Iterator`] over the same commands, which is the
    /// shape a `for` loop wants; this is the shape `?` wants.
    pub fn read(&mut self) -> Result<Option<Command>, Error> {
        loop {
            let Some(line) = self.take_line()? else {
                return Ok(None);
            };
            // Commands are separated by blank lines, and the optional newline
            // after a data block can look like one. Neither says anything.
            if line.is_empty() {
                continue;
            }
            return self.command(&line).map(Some);
        }
    }

    fn command(&mut self, line: &[u8]) -> Result<Command, Error> {
        if line == b"blob" {
            return self.blob().map(Command::Blob);
        }
        if line == b"done" {
            return Ok(Command::Done);
        }
        if let Some(reference) = after(line, b"commit ") {
            return self.commit(reference.to_vec()).map(Command::Commit);
        }
        if let Some(name) = after(line, b"tag ") {
            return self.tag(name.to_vec()).map(Command::Tag);
        }
        if let Some(reference) = after(line, b"reset ") {
            return self.reset(reference.to_vec()).map(Command::Reset);
        }
        if let Some(rest) = after(line, b"progress ") {
            return Ok(Command::Progress(rest.to_vec()));
        }
        if let Some(rest) = after(line, b"feature ") {
            return Ok(Command::Feature(rest.to_vec()));
        }
        if let Some(rest) = after(line, b"option ") {
            return Ok(Command::Option(rest.to_vec()));
        }
        // The commands a frontend uses to ask git a question. They cannot come
        // out of `git fast-export`, and reading one would mean answering it.
        for asked in [
            &b"checkpoint"[..],
            b"get-mark ",
            b"cat-blob ",
            b"ls ",
            b"alias",
        ] {
            if line.starts_with(asked) {
                let name = String::from_utf8_lossy(asked).trim_end().to_string();
                return Err(self.fault(format!(
                    "`{name}` is a command a frontend sends to git, and this reads \
                     what git sends — a stream containing it did not come from \
                     `git fast-export`"
                )));
            }
        }
        Err(self.fault(format!(
            "`{}` is not a command this stream has",
            String::from_utf8_lossy(&first_word(line))
        )))
    }

    fn blob(&mut self) -> Result<Blob, Error> {
        let mark = self.optional_mark()?;
        let original_oid = self.optional_field(b"original-oid ")?;
        let data = self.data()?;
        Ok(Blob {
            mark,
            original_oid: original_oid.map(|oid| String::from_utf8_lossy(&oid).into_owned()),
            data,
        })
    }

    fn commit(&mut self, reference: Vec<u8>) -> Result<Commit, Error> {
        let mark = self.optional_mark()?;
        let original_oid = self.optional_field(b"original-oid ")?;
        let author = match self.optional_field(b"author ")? {
            Some(line) => Some(self.person(&line)?),
            None => None,
        };
        let committer = match self.optional_field(b"committer ")? {
            Some(line) => self.person(&line)?,
            None => {
                return Err(self.fault(
                    "a commit must state a `committer` — git writes one on every \
                     commit, even when the author is the same person",
                ));
            }
        };
        let encoding = self.optional_field(b"encoding ")?;
        // After `encoding` and before the message, which is where git writes
        // it. A `gpgsig` with no data block after it is a truncated commit
        // rather than a signature this reader can skip.
        let signature = match self.optional_field(b"gpgsig ")? {
            Some(kind) => Some(Signature {
                kind,
                data: self.data()?,
            }),
            None => None,
        };
        let message = self.data()?;
        let from = match self.optional_field(b"from ")? {
            Some(line) => Some(self.data_ref(&line)?),
            None => None,
        };
        let mut merges = Vec::new();
        while let Some(line) = self.optional_field(b"merge ")? {
            merges.push(self.data_ref(&line)?);
        }
        let mut changes = Vec::new();
        while let Some(line) = self.take_line()? {
            if line.is_empty() {
                break;
            }
            match self.change(&line)? {
                Some(change) => changes.push(change),
                // Not a change: the commit ended, and this line starts
                // whatever comes next.
                None => {
                    self.put_back(line);
                    break;
                }
            }
        }
        Ok(Commit {
            reference,
            mark,
            original_oid: original_oid.map(|oid| String::from_utf8_lossy(&oid).into_owned()),
            author,
            committer,
            encoding,
            signature,
            message,
            from,
            merges,
            changes,
        })
    }

    fn change(&mut self, line: &[u8]) -> Result<Option<Change>, Error> {
        if line == b"deleteall" {
            return Ok(Some(Change::DeleteAll));
        }
        if let Some(rest) = after(line, b"M ") {
            let (mode, rest) = self.split_once(rest, "a `M` line")?;
            let mode = self.mode(mode)?;
            let (reference, rest) = self.split_once(rest, "a `M` line")?;
            let content = self.content(reference)?;
            let (path, _) = self.path(rest, false)?;
            return Ok(Some(Change::Modify {
                mode,
                content,
                path,
            }));
        }
        if let Some(rest) = after(line, b"D ") {
            let (path, _) = self.path(rest, false)?;
            return Ok(Some(Change::Delete { path }));
        }
        if let Some(rest) = after(line, b"R ") {
            let (source, destination) = self.two_paths(rest, "a `R` line")?;
            return Ok(Some(Change::Rename {
                source,
                destination,
            }));
        }
        if let Some(rest) = after(line, b"C ") {
            let (source, destination) = self.two_paths(rest, "a `C` line")?;
            return Ok(Some(Change::Copy {
                source,
                destination,
            }));
        }
        if let Some(rest) = after(line, b"N ") {
            let (reference, commit) = self.split_once(rest, "a `N` line")?;
            let content = self.content(reference)?;
            let commit = self.data_ref(commit)?;
            return Ok(Some(Change::Note { content, commit }));
        }
        Ok(None)
    }

    fn tag(&mut self, name: Vec<u8>) -> Result<Tag, Error> {
        let mark = self.optional_mark()?;
        let from = match self.optional_field(b"from ")? {
            Some(line) => self.data_ref(&line)?,
            None => {
                return Err(self.fault(
                    "a `tag` must state what it tags with `from` — a tag of nothing \
                     is not something git can be given",
                ));
            }
        };
        let original_oid = self.optional_field(b"original-oid ")?;
        let tagger = match self.optional_field(b"tagger ")? {
            Some(line) => Some(self.person(&line)?),
            None => None,
        };
        let message = self.data()?;
        Ok(Tag {
            name,
            mark,
            from,
            original_oid: original_oid.map(|oid| String::from_utf8_lossy(&oid).into_owned()),
            tagger,
            message,
        })
    }

    fn reset(&mut self, reference: Vec<u8>) -> Result<Reset, Error> {
        let from = match self.optional_field(b"from ")? {
            Some(line) => Some(self.data_ref(&line)?),
            None => None,
        };
        Ok(Reset { reference, from })
    }

    // -----------------------------------------------------------------------
    // The pieces a command is made of
    // -----------------------------------------------------------------------

    /// `data <n>` and the bytes after it, or `data <<DELIM` and the lines up to
    /// the delimiter.
    ///
    /// The counted form is what git writes and the only one that can carry
    /// arbitrary bytes; the delimited form is accepted because the format has
    /// it and a stream written by hand is likely to use it.
    fn data(&mut self) -> Result<Vec<u8>, Error> {
        let Some(line) = self.take_line()? else {
            return Err(self.fault("the stream ended where a `data` was expected"));
        };
        let Some(rest) = after(&line, b"data ") else {
            return Err(self.fault(format!(
                "expected `data`, found `{}`",
                String::from_utf8_lossy(&first_word(&line))
            )));
        };
        if let Some(delimiter) = after(rest, b"<<") {
            let delimiter = delimiter.to_vec();
            let mut out = Vec::new();
            loop {
                let Some(line) = self.take_line()? else {
                    return Err(self.fault(format!(
                        "the stream ended before the delimiter `{}` this `data` opened with",
                        String::from_utf8_lossy(&delimiter)
                    )));
                };
                if line == delimiter {
                    return Ok(out);
                }
                out.extend_from_slice(&line);
                out.push(b'\n');
            }
        }
        let count = String::from_utf8_lossy(rest);
        let count: usize = count.trim().parse().map_err(|_| {
            self.fault(format!(
                "`data {count}` does not state a byte count — it is either \
                 `data <n>` or `data <<DELIMITER`"
            ))
        })?;
        let mut out = vec![0; count];
        std::io::Read::read_exact(&mut self.input, &mut out).map_err(|e| {
            let short = e.kind() == io::ErrorKind::UnexpectedEof;
            self.fault_from(
                if short {
                    format!("the stream ended {count} bytes short of what this `data` promised")
                } else {
                    "could not read the bytes this `data` promised".into()
                },
                e,
            )
        })?;
        self.line += out.iter().filter(|&&byte| byte == b'\n').count();
        // The newline git writes after a data block is not part of it, and is
        // optional in the format. Take it if it is there.
        self.skip_optional_newline()?;
        Ok(out)
    }

    fn optional_mark(&mut self) -> Result<Option<Mark>, Error> {
        match self.optional_field(b"mark ")? {
            Some(line) => match self.data_ref(&line)? {
                DataRef::Mark(mark) => Ok(Some(mark)),
                DataRef::Oid(oid) => Err(self.fault(format!(
                    "`mark {oid}` is not a mark — a mark is `:` and a number"
                ))),
            },
            None => Ok(None),
        }
    }

    /// `:7`, or a hex object ID.
    fn data_ref(&mut self, text: &[u8]) -> Result<DataRef, Error> {
        if let Some(number) = after(text, b":") {
            let number = String::from_utf8_lossy(number);
            return number
                .trim()
                .parse()
                .map(|n| DataRef::Mark(Mark(n)))
                .map_err(|_| self.fault(format!("`:{number}` is not a mark number")));
        }
        let text = String::from_utf8_lossy(text).trim().to_string();
        if !text.is_empty() && text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(DataRef::Oid(text));
        }
        Err(self.fault(format!(
            "`{text}` names nothing — it is either `:` and a mark number, or an \
             object ID in hex"
        )))
    }

    fn content(&mut self, text: &[u8]) -> Result<Content, Error> {
        if text == b"inline" {
            return Ok(Content::Inline(self.data()?));
        }
        self.data_ref(text).map(Content::Named)
    }

    fn mode(&mut self, text: &[u8]) -> Result<Mode, Error> {
        Ok(match text {
            b"100644" | b"644" => Mode::File,
            b"100755" | b"755" => Mode::Executable,
            b"120000" => Mode::Symlink,
            b"160000" => Mode::Gitlink,
            b"040000" => Mode::Directory,
            other => {
                return Err(self.fault(format!(
                    "`{}` is not a mode git states — they are 100644, 100755, \
                     120000, 160000, and 040000",
                    String::from_utf8_lossy(other)
                )));
            }
        })
    }

    /// A path, raw or quoted. Returns what came after it, which is what a
    /// two-path line needs.
    fn path(&mut self, text: &[u8], stop_at_space: bool) -> Result<(Vec<u8>, usize), Error> {
        if text.first() == Some(&b'"') {
            return unquote(text).map_err(|e| self.fault(e.to_string()));
        }
        if stop_at_space {
            let end = text
                .iter()
                .position(|&byte| byte == b' ')
                .unwrap_or(text.len());
            return Ok((text[..end].to_vec(), end));
        }
        Ok((text.to_vec(), text.len()))
    }

    fn two_paths(&mut self, text: &[u8], what: &str) -> Result<(Vec<u8>, Vec<u8>), Error> {
        let (source, past) = self.path(text, true)?;
        let rest = text.get(past..).unwrap_or_default();
        let Some(rest) = after(rest, b" ") else {
            return Err(self.fault(format!(
                "{what} states one path and needs two — a source and a destination, \
                 the source quoted if it holds a space"
            )));
        };
        let (destination, _) = self.path(rest, false)?;
        Ok((source, destination))
    }

    fn person(&mut self, text: &[u8]) -> Result<Person, Error> {
        let shown = || String::from_utf8_lossy(text).into_owned();
        let malformed = |reader: &Self, why: &str| {
            reader.fault(format!(
                "`{}` is not a person and a time: {why} — the shape is \
                 `Name <email> <seconds> <offset>`",
                shown()
            ))
        };
        let close = text
            .iter()
            .rposition(|&byte| byte == b'>')
            .ok_or_else(|| malformed(self, "no `>` closing an address"))?;
        let open = text[..close]
            .iter()
            .rposition(|&byte| byte == b'<')
            .ok_or_else(|| malformed(self, "no `<` opening an address"))?;
        let name = trim_end(&text[..open]).to_vec();
        let email = text[open + 1..close].to_vec();

        let when = trim_start(text.get(close + 1..).unwrap_or_default());
        let split = when
            .iter()
            .position(|&byte| byte == b' ')
            .ok_or_else(|| malformed(self, "the time states no offset"))?;
        let seconds = String::from_utf8_lossy(&when[..split]);
        let seconds: i64 = seconds
            .trim()
            .parse()
            .map_err(|_| malformed(self, "the seconds are not a number"))?;
        let offset_minutes = self.offset(trim_start(&when[split..]), &malformed)?;
        Ok(Person {
            name,
            email,
            seconds,
            offset_minutes,
        })
    }

    /// `+0530` and `-0700`: a sign, two digits of hours, two of minutes.
    fn offset(
        &mut self,
        text: &[u8],
        malformed: &dyn Fn(&Self, &str) -> Error,
    ) -> Result<i32, Error> {
        let text = trim_end(text);
        if text.len() != 5 || !text[1..].iter().all(u8::is_ascii_digit) {
            return Err(malformed(self, "the offset is not `+HHMM` or `-HHMM`"));
        }
        let sign = match text[0] {
            b'+' => 1,
            b'-' => -1,
            _ => return Err(malformed(self, "the offset has no sign")),
        };
        let digits = |at: usize| i32::from(text[at] - b'0');
        let hours = digits(1) * 10 + digits(2);
        let minutes = digits(3) * 10 + digits(4);
        Ok(sign * (hours * 60 + minutes))
    }

    // -----------------------------------------------------------------------
    // Lines
    // -----------------------------------------------------------------------

    /// The next line if it starts with `prefix`, and nothing taken from the
    /// stream if it does not.
    fn optional_field(&mut self, prefix: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        let Some(line) = self.take_line()? else {
            return Ok(None);
        };
        match after(&line, prefix) {
            Some(rest) => Ok(Some(rest.to_vec())),
            None => {
                self.put_back(line);
                Ok(None)
            }
        }
    }

    fn take_line(&mut self) -> Result<Option<Vec<u8>>, Error> {
        if let Some(line) = self.pending.take() {
            return Ok(Some(line));
        }
        let mut line = Vec::new();
        let read = self
            .input
            .read_until(b'\n', &mut line)
            .map_err(|e| self.fault_from("could not read the stream".into(), e))?;
        if read == 0 {
            return Ok(None);
        }
        self.line += 1;
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        Ok(Some(line))
    }

    fn put_back(&mut self, line: Vec<u8>) {
        debug_assert!(self.pending.is_none(), "one line of lookahead, not two");
        self.pending = Some(line);
    }

    fn skip_optional_newline(&mut self) -> Result<(), Error> {
        debug_assert!(
            self.pending.is_none(),
            "a data block is read from the stream, not from lookahead"
        );
        let newline = match self.input.fill_buf() {
            Ok(peeked) => peeked.first() == Some(&b'\n'),
            Err(e) => return Err(self.fault_from("could not read the stream".into(), e)),
        };
        if newline {
            self.input.consume(1);
            self.line += 1;
        }
        Ok(())
    }

    fn fault(&self, message: impl Into<String>) -> Error {
        Error {
            line: self.line,
            message: message.into(),
            source: None,
        }
    }

    fn fault_from(&self, message: String, source: io::Error) -> Error {
        Error {
            line: self.line,
            message,
            source: Some(source),
        }
    }

    fn split_once<'a>(&self, text: &'a [u8], what: &str) -> Result<(&'a [u8], &'a [u8]), Error> {
        match text.iter().position(|&byte| byte == b' ') {
            Some(at) => Ok((&text[..at], &text[at + 1..])),
            None => Err(self.fault(format!(
                "{what} ended early: `{}`",
                String::from_utf8_lossy(text)
            ))),
        }
    }
}

/// Every command in the stream, stopping at the first one that will not read.
impl<R: BufRead> Iterator for Reader<R> {
    type Item = Result<Command, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.read().transpose()
    }
}

fn after<'a>(text: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    text.starts_with(prefix).then(|| &text[prefix.len()..])
}

fn first_word(line: &[u8]) -> Vec<u8> {
    let end = line
        .iter()
        .position(|&byte| byte == b' ')
        .unwrap_or(line.len());
    line[..end].to_vec()
}

fn trim_end(text: &[u8]) -> &[u8] {
    let mut end = text.len();
    while end > 0 && text[end - 1] == b' ' {
        end -= 1;
    }
    &text[..end]
}

fn trim_start(text: &[u8]) -> &[u8] {
    let mut at = 0;
    while at < text.len() && text[at] == b' ' {
        at += 1;
    }
    &text[at..]
}
