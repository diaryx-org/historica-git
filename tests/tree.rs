//! Replaying a stream into the tree at each commit.
//!
//! The corpus half of this is checked against what git itself reports: the
//! expectations below are `git ls-tree -r --format='%(objectmode) %(path)'` at
//! each commit of the history `tests/corpus/export/make.sh` builds. If the
//! replay and git disagree about what a commit left behind, the conversion
//! standing on it would write the wrong folder.

use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::rc::Rc;

use historigit::stream::{Command, Reader};
use historigit::tree::{Error, Held, Tree, Trees};

/// Every commit's tree, by the commit's message, which is how a person reading
/// the fixture would name them.
fn replay(bytes: &[u8]) -> BTreeMap<String, Rc<Tree>> {
    let mut trees = Trees::new();
    let mut by_message = BTreeMap::new();
    for command in Reader::new(Cursor::new(bytes)) {
        let command = command.expect("the stream reads");
        let name = match &command {
            Command::Commit(commit) => {
                Some(String::from_utf8_lossy(&commit.message).trim().to_string())
            }
            _ => None,
        };
        let tree = trees.apply(&command).expect("the stream replays");
        if let (Some(name), Some(tree)) = (name, tree) {
            by_message.insert(name, tree);
        }
    }
    by_message
}

/// The tree as `git ls-tree -r --format='%(objectmode) %(path)'` would list it.
fn listed(tree: &Tree) -> Vec<String> {
    tree.iter()
        .map(|(path, entry)| format!("{} {}", entry.mode.as_str(), String::from_utf8_lossy(path)))
        .collect()
}

fn bytes_at(tree: &Tree, path: &str) -> Vec<u8> {
    match &tree
        .get(path.as_bytes())
        .expect("the path is in the tree")
        .content
    {
        Held::Bytes(bytes) => bytes.to_vec(),
        held => panic!("expected bytes at {path}, found {held:?}"),
    }
}

fn replay_text(stream: &str) -> BTreeMap<String, Rc<Tree>> {
    replay(stream.as_bytes())
}

#[test]
fn the_replay_agrees_with_git_about_every_tree() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/export/all.fi");
    let trees = replay(&fs::read(corpus).expect("the corpus is checked in"));

    assert_eq!(
        listed(&trees["Start"]),
        [
            "100644 a.txt",
            "100644 photo.bin",
            "100644 sub/with space.txt",
        ]
    );
    assert_eq!(
        listed(&trees["Edit and link"]),
        [
            "100755 a.txt",
            "120000 link",
            "100644 photo.bin",
            "100644 sub/with space.txt",
        ]
    );
    assert_eq!(
        listed(&trees["Rename and delete"]),
        [
            "120000 link",
            "100755 renamed.txt",
            "100644 sub/with space.txt",
        ]
    );
    assert_eq!(
        listed(&trees["Side"]),
        [
            "100644 a.txt",
            "100644 photo.bin",
            "100644 side.txt",
            "100644 sub/with space.txt",
        ],
        "a branch is replayed from the commit it forked at, not from the stream's \
         previous commit"
    );
    assert_eq!(
        listed(&trees["Merge side"]),
        [
            "120000 link",
            "100755 renamed.txt",
            "100644 side.txt",
            "100644 sub/with space.txt",
        ],
        "a merge states its changes against its first parent"
    );
    assert_eq!(
        listed(&trees["Empty"]),
        listed(&trees["Merge side"]),
        "a commit that changed nothing leaves what it was given"
    );

    // The content travels with the tree, and a rename carries it.
    assert_eq!(bytes_at(&trees["Start"], "a.txt"), b"one\ntwo\n");
    assert_eq!(
        bytes_at(&trees["Rename and delete"], "renamed.txt"),
        b"one\ntwo\nthree\n"
    );
    // A symlink's content is where it points, which is how git stores one.
    assert_eq!(bytes_at(&trees["Edit and link"], "link"), b"a.txt");
    assert_eq!(
        bytes_at(&trees["Start"], "photo.bin"),
        b"\x00\x01\x02binary\n"
    );
}

#[test]
fn a_directory_moves_with_everything_under_it() {
    // Git has no directories, only paths that share a beginning — so `R sub
    // other` has to move `sub/deep/a.txt` and leave `subtle.txt` where it is.
    let trees = replay_text(
        "blob\n\
         mark :1\n\
         data 2\n\
         a\n\
         commit refs/heads/main\n\
         mark :2\n\
         committer Bo <bo@example.com> 100 +0000\n\
         data 5\n\
         first\n\
         M 100644 :1 sub/deep/a.txt\n\
         M 100644 :1 subtle.txt\n\
         \n\
         commit refs/heads/main\n\
         mark :3\n\
         committer Bo <bo@example.com> 100 +0000\n\
         data 6\n\
         second\n\
         from :2\n\
         R sub other\n\
         \n",
    );
    assert_eq!(
        listed(&trees["second"]),
        ["100644 other/deep/a.txt", "100644 subtle.txt"]
    );
}

#[test]
fn deleting_a_directory_deletes_what_is_under_it() {
    let trees = replay_text(
        "blob\n\
         mark :1\n\
         data 2\n\
         a\n\
         commit refs/heads/main\n\
         mark :2\n\
         committer Bo <bo@example.com> 100 +0000\n\
         data 5\n\
         first\n\
         M 100644 :1 sub/deep/a.txt\n\
         M 100644 :1 subtle.txt\n\
         \n\
         commit refs/heads/main\n\
         mark :3\n\
         committer Bo <bo@example.com> 100 +0000\n\
         data 6\n\
         second\n\
         from :2\n\
         D sub\n\
         \n",
    );
    assert_eq!(listed(&trees["second"]), ["100644 subtle.txt"]);
}

#[test]
fn a_commit_with_no_parent_stated_continues_its_branch() {
    // fast-import reads a commit that states no `from` as continuing the ref it
    // is committed to. A replay that started from nothing would silently drop
    // every file the branch already held.
    let trees = replay_text(
        "blob\n\
         mark :1\n\
         data 2\n\
         a\n\
         commit refs/heads/main\n\
         mark :2\n\
         committer Bo <bo@example.com> 100 +0000\n\
         data 5\n\
         first\n\
         M 100644 :1 kept.txt\n\
         \n\
         commit refs/heads/main\n\
         mark :3\n\
         committer Bo <bo@example.com> 100 +0000\n\
         data 6\n\
         second\n\
         M 100644 :1 added.txt\n\
         \n",
    );
    assert_eq!(
        listed(&trees["second"]),
        ["100644 added.txt", "100644 kept.txt"]
    );
}

/// Replay a stream that is expected not to.
fn refuse(stream: &str) -> Error {
    let mut trees = Trees::new();
    for command in Reader::new(Cursor::new(stream.as_bytes())) {
        let command = command.expect("the stream reads");
        match trees.apply(&command) {
            Ok(_) => continue,
            Err(error) => return error,
        }
    }
    panic!("the stream replayed, and was expected not to");
}

#[test]
fn a_stream_that_cannot_be_replayed_says_which_flag_wrote_it() {
    let commit = "commit refs/heads/main\n\
                  mark :2\n\
                  committer Bo <bo@example.com> 100 +0000\n\
                  data 5\n\
                  first\n";

    let missing = refuse(&format!("{commit}M 100644 :9 a.txt\n"));
    assert!(
        matches!(missing, Error::NoSuchBlob { .. }),
        "expected a missing blob, found {missing}"
    );

    // `--no-data` names content the stream did not carry, which is a perfectly
    // good stream for some other purpose and useless for a conversion.
    let no_data = refuse(&format!(
        "{commit}M 100644 0123456789abcdef0123456789abcdef01234567 a.txt\n"
    ));
    assert!(
        no_data.to_string().contains("--no-data"),
        "the message should name the flag that wrote the stream: {no_data}"
    );

    // `--full-tree` states whole subtrees.
    let full_tree = refuse(&format!("{commit}M 040000 :1 sub\n"));
    assert!(
        full_tree.to_string().contains("--full-tree"),
        "the message should name the flag that wrote the stream: {full_tree}"
    );

    // A parent the stream never carried.
    let orphan = refuse(
        "commit refs/heads/main\n\
         mark :2\n\
         committer Bo <bo@example.com> 100 +0000\n\
         data 5\n\
         first\n\
         from :7\n",
    );
    assert!(
        matches!(orphan, Error::NoSuchCommit { .. }),
        "expected a missing parent, found {orphan}"
    );
}
