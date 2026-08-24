//! A change ID, derived from the commit the corpus actually carries.
//!
//! The unit tests in `src/identity.rs` pin the derivation against a constant.
//! This pins the constant against the corpus, so that the two cannot drift: if
//! `make.sh` ever builds a different history, this is what says so.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use historigit::identity::Identity;
use historigit::stream::{Command, Reader};

#[test]
fn the_corpus_commits_convert_to_changes_that_can_be_read_across() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/export/all.fi");
    let bytes = fs::read(corpus).expect("the corpus is checked in");

    let commits: Vec<_> = Reader::new(Cursor::new(&bytes))
        .map(|command| command.expect("the stream reads"))
        .filter_map(|command| match command {
            Command::Commit(commit) => Some(commit),
            _ => None,
        })
        .collect();
    assert_eq!(commits.len(), 6, "the fixture history is six commits");

    // The fixture is pinned — fixed dates, fixed identities, signing off — so
    // its object IDs are the same on every machine that builds it. That is what
    // makes decision 0003's worked example a constant rather than a snapshot.
    let root = &commits[0];
    assert_eq!(
        root.original_oid.as_deref(),
        Some("980aeeabdf5024a43392620b11f1d14de03a0bb5"),
        "make.sh builds a history whose object IDs do not move"
    );
    let identity = Identity::of(root).expect("the corpus carries object IDs");
    assert_eq!(
        identity.change_id().to_string(),
        "qrzpllpomkuzxvpvwwqxtxzo",
        "the change is the object ID's first twelve bytes, in reversed hex"
    );

    // Every commit converts, and no two of them to the same change.
    let mut changes: Vec<String> = commits
        .iter()
        .map(|commit| {
            Identity::of(commit)
                .expect("every commit carries an object ID")
                .change_id()
                .to_string()
        })
        .collect();
    let count = changes.len();
    changes.sort();
    changes.dedup();
    assert_eq!(changes.len(), count, "two commits converted to one change");
}

#[test]
fn a_stream_without_object_ids_is_refused() {
    // The same history, exported without `--show-original-ids`, is a perfectly
    // good stream that decision 0003 cannot use.
    let stream = "commit refs/heads/main\n\
                  mark :1\n\
                  committer Bo <bo@example.com> 100 +0000\n\
                  data 5\n\
                  first\n";
    let commands: Vec<_> = Reader::new(Cursor::new(stream.as_bytes()))
        .map(|command| command.expect("the stream reads"))
        .collect();
    let Command::Commit(commit) = &commands[0] else {
        panic!("expected a commit");
    };
    let refused = Identity::of(commit).expect_err("a commit with no object ID");
    assert!(
        refused.to_string().contains("--show-original-ids"),
        "the message has to name the flag that would fix it: {refused}"
    );
}
