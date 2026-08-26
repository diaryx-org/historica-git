//! Writing a store out, and what comes back when git reads it.
//!
//! The corpus is converted into a store by `import` and written out again here,
//! so this exercises the whole bridge in both directions with no repository on
//! disk. What needs git installed is separated out and says so.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use historica_git::stream::{Command, Reader};
use historica_git::{export, import};

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/export/all.fi")
}

fn folder(name: &str) -> PathBuf {
    let at = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&at);
    at
}

/// The corpus, as a store.
fn store(name: &str) -> PathBuf {
    let at = folder(name);
    let stream = fs::File::open(corpus()).expect("the corpus is checked in");
    import::from_stream(std::io::BufReader::new(stream), &at).expect("the corpus converts");
    at
}

fn write(from: &Path) -> (Vec<u8>, export::Report) {
    let mut out = Vec::new();
    let report = export::to_stream(from, &mut out).expect("the store writes out");
    (out, report)
}

/// Decision 0004's whole claim, tested where it can be tested without git:
/// nothing about the stream depends on when or where it was written.
#[test]
fn one_store_writes_the_same_stream_every_time() {
    let at = store("write-twice");
    let (once, first) = write(&at);
    let (again, second) = write(&at);

    assert_eq!(once, again, "two runs over one store disagreed");
    assert_eq!(first.commits, second.commits);
    assert!(first.commits > 0, "the corpus has commits in it");
}

/// What is written is a stream, which is to say the reader accepts it.
#[test]
fn what_is_written_reads_back() {
    let at = store("write-reads");
    let (bytes, report) = write(&at);

    let commands = Reader::new(Cursor::new(&bytes))
        .collect::<Result<Vec<_>, _>>()
        .expect("what was written reads");

    let commits = commands
        .iter()
        .filter(|command| matches!(command, Command::Commit(_)))
        .count();
    assert_eq!(
        commits, report.commits,
        "the report and the stream disagree"
    );
    assert!(
        matches!(commands.last(), Some(Command::Done)),
        "a stream should say it ended rather than merely stop"
    );
}

/// Decision 0001, from this end: the conversion says what it could not carry,
/// and the corpus holds one of everything it cannot.
#[test]
fn the_conversion_says_what_did_not_cross() {
    let at = store("write-says");
    let (_, report) = write(&at);
    let said = report.uncarried.join("\n");
    assert!(
        said.contains("change ID"),
        "change IDs go nowhere and that is worth saying: {said}"
    );
}

/// A commit's identity is a fact about the revision and nothing else, so a
/// store written out twice into two repositories produces two identical sets of
/// object IDs. Needs git.
#[test]
fn a_store_written_twice_produces_one_repository() {
    let Some(_) = git() else { return };
    let at = store("write-repos");

    let first = folder("write-repo-a");
    let second = folder("write-repo-b");
    export::to_repository(&at, &first).expect("the first repository is written");
    export::to_repository(&at, &second).expect("the second repository is written");

    assert_eq!(
        commits(&first),
        commits(&second),
        "two conversions of one store disagreed about what the commits are"
    );
    assert!(!commits(&first).is_empty(), "something was written");
}

/// Whether git is installed, since decision 0002 makes it a dependency of the
/// conversion rather than of this crate.
fn git() -> Option<()> {
    std::process::Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()
        .filter(std::process::ExitStatus::success)
        .map(|_| ())
}

fn commits(at: &Path) -> Vec<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(at)
        .args(["rev-list", "--all", "--reverse"])
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Decision 0004's headline, end to end: a repository historica can hold
/// entirely converts to a store and back to *the same commits*.
///
/// The repository is built here rather than checked in, because what is being
/// tested is agreement with the git on this machine. Everything in it is a fact
/// historica has a place for — one person, no signature, no committer distinct
/// from the author — which is exactly the set decision 0004 says round-trips.
/// What is left out is left out on purpose, and each omission is a line in that
/// decision's account of what does not cross.
#[test]
fn a_repository_historica_can_hold_converts_back_to_itself() {
    let Some(_) = git() else { return };

    let origin = folder("round-trip-origin");
    fs::create_dir_all(&origin).expect("a fresh directory");
    build(&origin);
    let before = commits(&origin);
    assert_eq!(
        before.len(),
        4,
        "the repository this test builds has 4 commits"
    );

    let store = folder("round-trip-store");
    import::from_repository(&origin, &store).expect("the repository imports");

    let back = folder("round-trip-back");
    export::to_repository(&store, &back).expect("the store writes out");

    assert_eq!(
        commits(&back),
        before,
        "a commit is supposed to be a function of the revision it came from"
    );
}

/// A history holding one of everything historica has a place for.
fn build(at: &Path) {
    // One person, one moment, so the commits do not depend on when the test ran
    // or on whose machine ran it.
    let as_committer = |who: (&str, &str), args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(at)
            .args(args)
            .env("GIT_AUTHOR_DATE", "2026-01-02T03:04:05-07:00")
            .env("GIT_COMMITTER_DATE", "2026-01-02T03:04:05-07:00")
            .env("GIT_AUTHOR_NAME", "Ada")
            .env("GIT_AUTHOR_EMAIL", "ada@example.com")
            .env("GIT_COMMITTER_NAME", who.0)
            .env("GIT_COMMITTER_EMAIL", who.1)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    };
    let run = |args: &[&str]| as_committer(("Ada", "ada@example.com"), args);

    run(&["init", "--quiet", "."]);
    // Not the machine's, whatever the machine's says: a signed commit is a
    // fact historica has nowhere to put, and this test is about the set that
    // crosses whole.
    run(&["config", "commit.gpgsign", "false"]);

    fs::create_dir_all(at.join("sub")).expect("a subdirectory");
    fs::write(at.join("a.txt"), "one\n").expect("a file");
    fs::write(at.join("sub/with space.txt"), "nested\n").expect("a file");
    fs::write(at.join("photo.bin"), [0u8, 1, 2, b'b', b'i', b'n']).expect("a file");
    fs::write(at.join("run.sh"), "#!/bin/sh\necho hi\n").expect("a file");
    executable(&at.join("run.sh"));
    run(&["add", "-A"]);
    run(&["commit", "--quiet", "-m", "Start"]);

    fs::write(at.join("a.txt"), "one\ntwo\n").expect("a file");
    run(&["commit", "--quiet", "-a", "-m", "Edit a"]);

    run(&["mv", "a.txt", "b.txt"]);
    run(&["commit", "--quiet", "-m", "Rename a to b"]);

    // A committer who is not the author, which historica records one of and
    // which is therefore a `git.committer` header rather than a lost fact.
    // Without it this commit would not be the commit it came from.
    fs::write(at.join("b.txt"), "one\ntwo\nthree\n").expect("a file");
    as_committer(
        ("Bo", "bo@example.com"),
        &["commit", "--quiet", "-a", "-m", "Applied from a patch"],
    );
}

#[cfg(unix)]
fn executable(at: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut mode = fs::metadata(at).expect("the file exists").permissions();
    mode.set_mode(0o755);
    fs::set_permissions(at, mode).expect("the bit is set");
}

#[cfg(not(unix))]
fn executable(_: &Path) {}
