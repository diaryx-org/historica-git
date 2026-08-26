//! The reader, against a stream git actually wrote.
//!
//! `tests/corpus/export/all.fi` is `git fast-export --all -M` over the history
//! `make.sh` builds: a rename, a merge, a symlink, an executable file, a path
//! with a space in it, a file of bytes that is not text, an empty commit, and
//! both kinds of tag. It is checked in byte-exact, because a test that builds
//! its own input tests the machine it runs on as much as the code.
//!
//! `invalid/` is the other half of the specification. Each file is refused for
//! one stated reason, which is its name.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use historica_git::stream::{Change, Command, Content, DataRef, Mark, Mode, Reader, Writer};

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/export")
}

fn read_all(bytes: &[u8]) -> Vec<Command> {
    Reader::new(Cursor::new(bytes))
        .collect::<Result<Vec<_>, _>>()
        .expect("the corpus stream reads")
}

fn mark(n: u64) -> DataRef {
    DataRef::Mark(Mark(n))
}

#[test]
fn the_export_reads_as_the_history_it_came_from() {
    let bytes = fs::read(corpus().join("all.fi")).expect("the corpus is checked in");
    let commands = read_all(&bytes);

    // Three blobs, then the branch the first commit lands on.
    let Command::Blob(first) = &commands[0] else {
        panic!(
            "expected the first command to be a blob, found {:?}",
            commands[0]
        );
    };
    assert_eq!(first.mark, Some(Mark(1)));
    assert_eq!(first.data, b"one\ntwo\n");

    let Command::Blob(photo) = &commands[1] else {
        panic!("expected a blob");
    };
    assert_eq!(
        photo.data, b"\x00\x01\x02binary\n",
        "a blob carries bytes, NULs included, rather than text"
    );

    let Command::Reset(reset) = &commands[3] else {
        panic!("expected a reset before the first commit");
    };
    assert_eq!(reset.reference, b"refs/heads/side");
    assert_eq!(reset.from, None);

    // The root commit: two people, three files, one of them at a quoted path.
    let Command::Commit(start) = &commands[4] else {
        panic!("expected the root commit");
    };
    assert_eq!(start.reference, b"refs/heads/side");
    assert_eq!(start.mark, Some(Mark(4)));
    assert_eq!(start.message, b"Start\n");
    assert_eq!(start.from, None, "a root commit has no parent");
    assert!(start.merges.is_empty());

    let author = start.author.as_ref().expect("git wrote an author");
    assert_eq!(author.name, b"Ada");
    assert_eq!(author.email, b"ada@example.com");
    assert_eq!(author.seconds, 1_767_348_245);
    assert_eq!(author.offset_minutes, -420, "-0700 is seven hours west");
    assert_eq!(start.committer.name, b"Bo");
    assert_eq!(start.committer.seconds, 1_767_348_246);

    assert_eq!(
        start.changes,
        vec![
            Change::Modify {
                mode: Mode::File,
                content: Content::Named(mark(1)),
                path: b"a.txt".to_vec(),
            },
            Change::Modify {
                mode: Mode::File,
                content: Content::Named(mark(2)),
                path: b"photo.bin".to_vec(),
            },
            Change::Modify {
                mode: Mode::File,
                content: Content::Named(mark(3)),
                // Git quoted this path because it holds a space. What the
                // commit did is put a file at the path without the quotes.
                path: b"sub/with space.txt".to_vec(),
            },
        ]
    );

    // The modes beyond an ordinary file.
    let Command::Commit(link) = &commands[7] else {
        panic!("expected the commit that adds the link");
    };
    assert_eq!(link.from, Some(mark(4)));
    assert_eq!(
        link.changes,
        vec![
            Change::Modify {
                mode: Mode::Executable,
                content: Content::Named(mark(5)),
                path: b"a.txt".to_vec(),
            },
            Change::Modify {
                mode: Mode::Symlink,
                content: Content::Named(mark(6)),
                path: b"link".to_vec(),
            },
        ]
    );

    // `-M` states a rename as a rename, which is the shape historica records
    // in too — decision 0008 there.
    let Command::Commit(renamed) = &commands[8] else {
        panic!("expected the rename commit");
    };
    assert_eq!(
        renamed.changes,
        vec![
            Change::Rename {
                source: b"a.txt".to_vec(),
                destination: b"renamed.txt".to_vec(),
            },
            Change::Delete {
                path: b"photo.bin".to_vec(),
            },
        ]
    );

    // A merge names its first parent with `from` and the rest with `merge`.
    let merge = commands
        .iter()
        .find_map(|command| match command {
            Command::Commit(commit) if commit.message == b"Merge side\n" => Some(commit),
            _ => None,
        })
        .expect("the merge is in the stream");
    assert_eq!(merge.from, Some(mark(8)));
    assert_eq!(merge.merges, vec![mark(10)]);

    // A commit that changed nothing says so by stating nothing.
    let empty = commands
        .iter()
        .find_map(|command| match command {
            Command::Commit(commit) if commit.message == b"Empty\n" => Some(commit),
            _ => None,
        })
        .expect("the empty commit is in the stream");
    assert!(empty.changes.is_empty());
    assert_eq!(empty.mark, Some(Mark(12)));

    // A lightweight tag is a ref that moved; an annotated one is its own object.
    let Command::Reset(light) = &commands[commands.len() - 2] else {
        panic!("expected the lightweight tag as a reset");
    };
    assert_eq!(light.reference, b"refs/tags/light");
    assert_eq!(light.from, Some(mark(12)));

    let Command::Tag(annotated) = commands.last().expect("the stream is not empty") else {
        panic!("expected the annotated tag last");
    };
    assert_eq!(annotated.name, b"annotated");
    assert_eq!(annotated.from, mark(12));
    assert_eq!(annotated.message, b"An annotated tag\n");
    assert_eq!(
        annotated.tagger.as_ref().expect("git wrote a tagger").name,
        b"Bo"
    );
}

#[test]
fn every_invalid_stream_is_refused_by_line() {
    let directory = corpus().join("invalid");
    let mut refused = 0;
    let mut entries: Vec<_> = fs::read_dir(&directory)
        .expect("the invalid corpus is checked in")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "fi"))
        .collect();
    entries.sort();

    for path in &entries {
        let bytes = fs::read(path).expect("a fixture reads");
        let outcome: Result<Vec<_>, _> = Reader::new(Cursor::new(&bytes)).collect();
        let name = path.file_name().expect("a fixture has a name");
        let error = match outcome {
            Ok(commands) => panic!("{name:?} was accepted, and reads as {commands:?}"),
            Err(error) => error,
        };
        assert!(
            error.line > 0,
            "{name:?} was refused without naming a line: {error}"
        );
        // The message is what a person reads when a conversion stops, so it has
        // to say more than that something was wrong.
        assert!(
            error.to_string().len() > "line 1: no".len(),
            "{name:?} was refused with nothing to act on: {error}"
        );
        refused += 1;
    }
    assert_eq!(refused, entries.len());
    assert!(refused >= 10, "the invalid corpus lost cases: {refused}");
}

#[test]
fn a_stream_is_read_one_command_at_a_time() {
    // The whole point of reading from a `BufRead`: a repository's export is
    // never held at once. Nothing here proves the memory, but it does prove the
    // reader does not need the end of the stream to hand back the beginning.
    let bytes = fs::read(corpus().join("all.fi")).expect("the corpus is checked in");
    let mut reader = Reader::new(Cursor::new(&bytes));
    let first = reader.read().expect("the first command reads");
    assert!(matches!(first, Some(Command::Blob(_))));
    let rest = reader.count();
    assert_eq!(rest + 1, read_all(&bytes).len());
}

/// The writer, against the same stream: what git wrote reads and writes back
/// unchanged.
///
/// This is the whole specification for [`Writer`]. Decision 0004 makes a commit
/// a function of the revision, and the way that claim is ever checked is by
/// comparing a stream this crate wrote with one git wrote — which is worth
/// nothing if the two differ in a quoting rule or a trailing newline. So the
/// corpus is read into commands and written back, and the bytes are the test.
#[test]
fn a_stream_git_wrote_is_written_back_byte_for_byte() {
    let bytes = fs::read(corpus().join("all.fi")).expect("the corpus is checked in");
    let commands = read_all(&bytes);

    let mut writer = Writer::new(Vec::new());
    for command in &commands {
        writer.write(command).expect("a vector takes bytes");
    }
    let written = writer.into_inner();

    if written != bytes {
        let at = written
            .iter()
            .zip(&bytes)
            .position(|(a, b)| a != b)
            .unwrap_or(written.len().min(bytes.len()));
        let from = at.saturating_sub(60);
        panic!(
            "the stream differs from git\'s at byte {at}\n\
             wrote:  {:?}\n\
             git had: {:?}",
            String::from_utf8_lossy(&written[from..(at + 60).min(written.len())]),
            String::from_utf8_lossy(&bytes[from..(at + 60).min(bytes.len())]),
        );
    }
}
