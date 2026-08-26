//! Writing the stream `read` parses.
//!
//! The same vocabulary in the other direction, and deliberately no more than
//! that: a [`Writer`] takes the [`Command`]s a [`Reader`](super::Reader) yields
//! and spells them the way git spells them. Nothing here decides what a commit
//! should contain — that is [`crate::export`]'s question, and keeping it out of
//! this module is what keeps "what did git say" and "what should we say"
//! separable, exactly as the reader's own doc comment argues.
//!
//! Byte-exactness is the point rather than a nicety. Decision 0004 makes a
//! commit a function of the revision, and the way that claim is checked is by
//! writing a stream and comparing it with the one git wrote, so every place
//! git had a choice about spelling is resolved the way git resolves it.

use std::io::{self, Write};

use super::quote::quote;
use super::{Blob, Change, Command, Commit, Content, DataRef, Person, Reset, Tag};

/// Writes fast-import commands to whatever will take bytes.
///
/// The counterpart of [`Reader`](super::Reader): what one yields, the other
/// writes, and a stream read and written again is the stream that arrived.
#[derive(Debug)]
pub struct Writer<W> {
    out: W,
}

impl<W: Write> Writer<W> {
    /// A writer over `out`.
    pub fn new(out: W) -> Self {
        Self { out }
    }

    /// Hand back what was being written to.
    pub fn into_inner(self) -> W {
        self.out
    }

    /// Push whatever is buffered at whoever is downstream.
    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    /// Write one command.
    pub fn write(&mut self, command: &Command) -> io::Result<()> {
        match command {
            Command::Blob(blob) => self.blob(blob),
            Command::Commit(commit) => self.commit(commit),
            Command::Tag(tag) => self.tag(tag),
            Command::Reset(reset) => self.reset(reset),
            Command::Progress(line) => self.line(b"progress", line),
            Command::Feature(line) => self.line(b"feature", line),
            Command::Option(line) => self.line(b"option", line),
            Command::Done => self.out.write_all(b"done\n"),
        }
    }

    fn blob(&mut self, blob: &Blob) -> io::Result<()> {
        self.out.write_all(b"blob\n")?;
        if let Some(mark) = blob.mark {
            writeln!(self.out, "mark :{}", mark.0)?;
        }
        if let Some(oid) = &blob.original_oid {
            writeln!(self.out, "original-oid {oid}")?;
        }
        self.data(&blob.data)?;
        // A blob ends here, so the newline git writes after the bytes is this
        // command's terminator. A commit's message gets no such newline: see
        // `data` below.
        self.out.write_all(b"\n")
    }

    fn commit(&mut self, commit: &Commit) -> io::Result<()> {
        self.out.write_all(b"commit ")?;
        self.out.write_all(&commit.reference)?;
        self.out.write_all(b"\n")?;
        if let Some(mark) = commit.mark {
            writeln!(self.out, "mark :{}", mark.0)?;
        }
        if let Some(oid) = &commit.original_oid {
            writeln!(self.out, "original-oid {oid}")?;
        }
        if let Some(author) = &commit.author {
            self.person(b"author", author)?;
        }
        self.person(b"committer", &commit.committer)?;
        if let Some(encoding) = &commit.encoding {
            self.out.write_all(b"encoding ")?;
            self.out.write_all(encoding)?;
            self.out.write_all(b"\n")?;
        }
        if let Some(signature) = &commit.signature {
            self.out.write_all(b"gpgsig ")?;
            self.out.write_all(&signature.kind)?;
            self.out.write_all(b"\n")?;
            self.data(&signature.data)?;
            // Git ends the signature's data block with a newline, since the
            // message's `data` line has to start one.
            self.out.write_all(b"\n")?;
        }
        // No newline after the message. Git writes none — a message that does
        // not end in one runs straight into the first `M` on the same line,
        // which is observable in `git fast-export` output and which
        // `fast-import` reads correctly because the count already said where
        // the message stopped. The commit's terminator below is the newline.
        self.data(&commit.message)?;
        if let Some(from) = &commit.from {
            self.out.write_all(b"from ")?;
            self.reference(from)?;
        }
        for merge in &commit.merges {
            self.out.write_all(b"merge ")?;
            self.reference(merge)?;
        }
        for change in &commit.changes {
            self.change(change)?;
        }
        // The blank line that ends a commit. Git writes one and fast-import
        // takes it as the end of the change list.
        self.out.write_all(b"\n")
    }

    fn tag(&mut self, tag: &Tag) -> io::Result<()> {
        self.out.write_all(b"tag ")?;
        self.out.write_all(&tag.name)?;
        self.out.write_all(b"\n")?;
        if let Some(mark) = tag.mark {
            writeln!(self.out, "mark :{}", mark.0)?;
        }
        self.out.write_all(b"from ")?;
        self.reference(&tag.from)?;
        // After `from` rather than after `mark`, because that is where git
        // writes it on a tag and nowhere else.
        if let Some(oid) = &tag.original_oid {
            writeln!(self.out, "original-oid {oid}")?;
        }
        if let Some(tagger) = &tag.tagger {
            self.person(b"tagger", tagger)?;
        }
        self.data(&tag.message)?;
        self.out.write_all(b"\n")
    }

    fn reset(&mut self, reset: &Reset) -> io::Result<()> {
        self.out.write_all(b"reset ")?;
        self.out.write_all(&reset.reference)?;
        self.out.write_all(b"\n")?;
        // A reset that only names a ref ends there: git writes no blank line
        // after one, and the corpus is where that is checked.
        match &reset.from {
            Some(from) => {
                self.out.write_all(b"from ")?;
                self.reference(from)?;
                self.out.write_all(b"\n")
            }
            None => Ok(()),
        }
    }

    fn change(&mut self, change: &Change) -> io::Result<()> {
        match change {
            Change::Modify {
                mode,
                content,
                path,
            } => {
                write!(self.out, "M {} ", mode.as_str())?;
                match content {
                    Content::Named(named) => {
                        self.named(named)?;
                        self.out.write_all(b" ")?;
                        self.out.write_all(&quote(path, true))?;
                        self.out.write_all(b"\n")
                    }
                    Content::Inline(bytes) => {
                        self.out.write_all(b"inline ")?;
                        self.out.write_all(&quote(path, true))?;
                        self.out.write_all(b"\n")?;
                        self.data(bytes)?;
                        self.out.write_all(b"\n")
                    }
                }
            }
            Change::Delete { path } => {
                self.out.write_all(b"D ")?;
                self.out.write_all(&quote(path, true))?;
                self.out.write_all(b"\n")
            }
            Change::Rename {
                source,
                destination,
            } => self.pair(b"R", source, destination),
            Change::Copy {
                source,
                destination,
            } => self.pair(b"C", source, destination),
            Change::DeleteAll => self.out.write_all(b"deleteall\n"),
            Change::Note { content, commit } => {
                self.out.write_all(b"N ")?;
                match content {
                    Content::Named(named) => {
                        self.named(named)?;
                        self.out.write_all(b" ")?;
                        self.named(commit)?;
                        self.out.write_all(b"\n")
                    }
                    Content::Inline(bytes) => {
                        self.out.write_all(b"inline ")?;
                        self.named(commit)?;
                        self.out.write_all(b"\n")?;
                        self.data(bytes)?;
                        self.out.write_all(b"\n")
                    }
                }
            }
        }
    }

    fn pair(&mut self, verb: &[u8], source: &[u8], destination: &[u8]) -> io::Result<()> {
        self.out.write_all(verb)?;
        self.out.write_all(b" ")?;
        self.out.write_all(&quote(source, true))?;
        self.out.write_all(b" ")?;
        self.out.write_all(&quote(destination, true))?;
        self.out.write_all(b"\n")
    }

    /// `data <count>` and exactly that many bytes, and nothing after them.
    ///
    /// The newline that follows a data block is the *command's*, not the
    /// block's, which is the distinction git draws and which a writer has to
    /// draw the same way to be diffable against git's own output: a blob and a
    /// tag end at their data and get one, and a commit's message is followed
    /// by its file changes and gets none.
    fn data(&mut self, bytes: &[u8]) -> io::Result<()> {
        writeln!(self.out, "data {}", bytes.len())?;
        self.out.write_all(bytes)
    }

    fn person(&mut self, role: &[u8], person: &Person) -> io::Result<()> {
        self.out.write_all(role)?;
        self.out.write_all(b" ")?;
        self.out.write_all(&person.name)?;
        self.out.write_all(b" <")?;
        self.out.write_all(&person.email)?;
        write!(self.out, "> {} ", person.seconds)?;
        self.out
            .write_all(offset(person.offset_minutes).as_bytes())?;
        self.out.write_all(b"\n")
    }

    fn reference(&mut self, named: &DataRef) -> io::Result<()> {
        self.named(named)?;
        self.out.write_all(b"\n")
    }

    fn named(&mut self, named: &DataRef) -> io::Result<()> {
        match named {
            DataRef::Mark(mark) => write!(self.out, ":{}", mark.0),
            DataRef::Oid(oid) => self.out.write_all(oid.as_bytes()),
        }
    }

    fn line(&mut self, verb: &[u8], rest: &[u8]) -> io::Result<()> {
        self.out.write_all(verb)?;
        self.out.write_all(b" ")?;
        self.out.write_all(rest)?;
        self.out.write_all(b"\n")
    }
}

/// Minutes east of UTC, as git's `+HHMM`.
fn offset(minutes: i32) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let minutes = minutes.abs();
    format!("{sign}{:02}{:02}", minutes / 60, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::Mode;

    fn written(command: &Command) -> Vec<u8> {
        let mut writer = Writer::new(Vec::new());
        writer.write(command).expect("a vector takes bytes");
        writer.into_inner()
    }

    #[test]
    fn an_offset_is_spelled_the_way_git_spells_one() {
        assert_eq!(offset(0), "+0000");
        assert_eq!(offset(-420), "-0700");
        assert_eq!(offset(330), "+0530");
    }

    #[test]
    fn a_blob_carries_its_count_and_a_newline_after_it() {
        let command = Command::Blob(Blob {
            mark: Some(crate::stream::Mark(1)),
            original_oid: None,
            data: b"one\ntwo\n".to_vec(),
        });
        assert_eq!(written(&command), b"blob\nmark :1\ndata 8\none\ntwo\n\n");
    }

    /// A signed commit, in the shape `git fast-export --signed-commits=verbatim`
    /// writes one, read and written back unchanged.
    ///
    /// Built here rather than added to the corpus because signing needs a key,
    /// and a test that needs a key is a test that does not run. The counts are
    /// computed rather than written down, since a hand-counted `data` line
    /// tests the arithmetic and not the code.
    #[test]
    fn a_signed_commit_survives_being_read_and_written() {
        let armour = "-----BEGIN PGP SIGNATURE-----\n\nabcd/+=\n-----END PGP SIGNATURE-----";
        let message = "Start\n";
        let stream = format!(
            "commit refs/heads/main\n\
             mark :1\n\
             author Ada <ada@example.com> 1767348245 -0700\n\
             committer Bo <bo@example.com> 1767348246 -0700\n\
             gpgsig sha1 openpgp\n\
             data {}\n{armour}\n\
             data {}\n{message}\
             M 100644 :2 a.txt\n\n",
            armour.len(),
            message.len(),
        )
        .into_bytes();

        let commands = crate::stream::Reader::new(std::io::Cursor::new(&stream))
            .collect::<Result<Vec<_>, _>>()
            .expect("a signed commit reads");
        let Some(Command::Commit(commit)) = commands.first() else {
            panic!("expected a commit, got {commands:?}");
        };
        let signature = commit
            .signature
            .as_ref()
            .expect("the signature came across");
        assert_eq!(signature.kind, b"sha1 openpgp");
        assert_eq!(signature.data, armour.as_bytes());

        let mut writer = Writer::new(Vec::new());
        for command in &commands {
            writer.write(command).expect("a vector takes bytes");
        }
        assert_eq!(
            writer.into_inner(),
            stream,
            "a signed commit did not write back as it read"
        );
    }

    #[test]
    fn a_path_holding_a_space_is_quoted_as_git_quotes_it() {
        let command = Command::Commit(Commit {
            reference: b"refs/heads/main".to_vec(),
            mark: None,
            original_oid: None,
            author: None,
            committer: Person {
                name: b"Ada".to_vec(),
                email: b"ada@example.com".to_vec(),
                seconds: 1767348245,
                offset_minutes: -420,
            },
            encoding: None,
            signature: None,
            message: b"m\n".to_vec(),
            from: None,
            merges: Vec::new(),
            changes: vec![Change::Modify {
                mode: Mode::File,
                content: Content::Named(DataRef::Mark(crate::stream::Mark(3))),
                path: b"sub/with space.txt".to_vec(),
            }],
        });
        let out = written(&command);
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("M 100644 :3 \"sub/with space.txt\"\n"),
            "{text}"
        );
        assert!(text.ends_with("\n\n"), "a commit ends with a blank line");
    }
}
