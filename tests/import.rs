//! Converting the corpus into a store.
//!
//! The corpus is a stream git wrote, so this is the whole bridge end to end:
//! read, replay, materialise, record. It needs no git installed — the stream is
//! checked in — which is the property decision 0002 said the stream form buys.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use historica::store::{STORE_DIR, Store};
use historica_git::import;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/export/all.fi")
}

/// A folder of this test's own, emptied first so a rerun is a fresh run.
fn folder(name: &str) -> PathBuf {
    let at = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&at);
    at
}

fn convert(into: &Path) -> import::Report {
    let stream = fs::File::open(corpus()).expect("the corpus is checked in");
    import::from_stream(std::io::BufReader::new(stream), into).expect("the corpus converts")
}

/// Every file under `at`, by its path from `at`, so two conversions can be
/// compared byte for byte.
fn contents(at: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, at: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(at).expect("a directory this test just wrote") {
            let path = entry.expect("a directory entry").path();
            let name = path
                .strip_prefix(root)
                .expect("under the root")
                .to_string_lossy()
                .into_owned();
            if fs::symlink_metadata(&path).expect("it is there").is_dir() {
                walk(root, &path, into);
            } else {
                into.insert(name, fs::read(&path).unwrap_or_default());
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(at, at, &mut found);
    found
}

#[test]
fn the_corpus_converts_into_a_store_that_checks() {
    let into = folder("corpus");
    let report = convert(&into);

    assert_eq!(report.commits, 6, "the fixture history is six commits");
    assert_eq!(
        report.revisions, 5,
        "five of them changed something, or joined two lines of work"
    );
    assert_eq!(
        report.empty, 1,
        "the empty commit has no revision; what stood on it stands on its parent"
    );

    // Decision 0006: a branch and a lightweight tag are both only pointers, so
    // both cross as bookmarks. `side` is a branch the fixture leaves behind a
    // merge, and it is named even though nothing stands on it.
    assert_eq!(
        report.bookmarks,
        // In the order the refs sort, which is the whole ref name: the two
        // under `refs/heads/` before the one under `refs/tags/`.
        vec!["main".to_owned(), "side".to_owned(), "light".to_owned()],
        "every ref that is only a pointer should have become a bookmark"
    );

    // Decision 0001: what could not cross is stated rather than dropped. The
    // annotated tag is the one ref that does not, because it is an object with
    // a tagger and a message rather than a pointer.
    let said = report.uncarried.join("\n");
    for expected in ["committer", "annotated tag", "changed nothing"] {
        assert!(
            said.contains(expected),
            "the report should say what happened to {expected}:\n{said}"
        );
    }
    assert!(
        !report.bookmarks.contains(&"annotated".to_owned()),
        "an annotated tag should not become a bookmark: {:?}",
        report.bookmarks
    );

    // The store historica itself is willing to stand behind.
    let checked = Store::check(into.join(STORE_DIR));
    let errors: Vec<String> = checked
        .errors()
        .map(|finding| format!("{finding:?}"))
        .collect();
    assert!(
        errors.is_empty(),
        "the converted store does not check: {errors:?}"
    );
}

#[test]
fn the_folder_ends_as_the_last_commit_left_it() {
    let into = folder("folder");
    convert(&into);

    let mut paths: Vec<String> = fs::read_dir(&into)
        .expect("the folder is there")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name != STORE_DIR)
        .collect();
    paths.sort();
    assert_eq!(paths, ["link", "renamed.txt", "side.txt", "sub"]);

    assert_eq!(
        fs::read_to_string(into.join("renamed.txt")).expect("the renamed file"),
        "one\ntwo\nthree\n"
    );
    assert_eq!(
        fs::read_link(into.join("link")).expect("a symbolic link"),
        Path::new("a.txt"),
        "a link is materialised as a link, not as a copy of what it points at"
    );
    // `photo.bin` was deleted two commits before the end, and the folder is the
    // tree the last commit describes rather than everything it ever held.
    assert!(!into.join("photo.bin").exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(into.join("renamed.txt"))
            .expect("the renamed file")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "the executable bit crossed");
    }
}

#[test]
fn converting_twice_produces_the_same_store() {
    // The whole of decision 0003, in one assertion. Change IDs come from the
    // commits rather than from entropy, so two conversions of one repository
    // are one history — byte for byte, on disk, with no comparison of models in
    // between.
    let one = folder("twice-one");
    let other = folder("twice-other");
    convert(&one);
    convert(&other);

    let left = contents(&one.join(STORE_DIR));
    let right = contents(&other.join(STORE_DIR));

    // `cache/` is disposable by historica's decision 0003 and is nobody's
    // interface, so it is not part of what two conversions have to agree on.
    let shed = |held: BTreeMap<String, Vec<u8>>| -> BTreeMap<String, Vec<u8>> {
        held.into_iter()
            .filter(|(path, _)| !path.starts_with("cache"))
            .collect()
    };
    let (left, right) = (shed(left), shed(right));

    assert!(!left.is_empty(), "the conversion wrote nothing");
    assert_eq!(
        left.keys().collect::<Vec<_>>(),
        right.keys().collect::<Vec<_>>(),
        "two conversions wrote different files"
    );
    for (path, bytes) in &left {
        assert_eq!(
            bytes, &right[path],
            "two conversions disagree about the bytes of {path}"
        );
    }
}

#[test]
fn a_folder_holding_anything_is_refused() {
    let into = folder("occupied");
    fs::create_dir_all(&into).expect("a folder");
    fs::write(into.join("mine.txt"), "a file a person put here").expect("a file");

    let stream = fs::File::open(corpus()).expect("the corpus is checked in");
    let refused = import::from_stream(std::io::BufReader::new(stream), &into)
        .expect_err("a folder with something in it");
    assert!(
        refused.to_string().contains("mine.txt"),
        "the refusal should name what is in the way: {refused}"
    );
    assert!(
        into.join("mine.txt").exists(),
        "a refused conversion must not have removed anything"
    );
}

/// A ref whose name historica will not hold is reported, not dropped and not
/// fatal.
///
/// Built as a stream rather than as a repository on purpose. Git on macOS
/// precomposes a ref name before it stores one — `core.precomposeunicode` — so
/// a branch created here with a decomposed `é` arrives already normalised and
/// this path is never reached. On a machine where git does not precompose, it
/// is: historica requires NFC and git does not normalise at all, which makes
/// this the one refusal whose reachability depends on where the conversion runs.
///
/// What it must do is the same either way. The ref does not become a bookmark,
/// the conversion finishes, every commit is still in the store, and the report
/// says which rule the name broke in historica's own words.
#[test]
fn a_ref_historica_will_not_name_is_reported_rather_than_dropped() {
    // `cafe` and a combining acute: NFC's decomposition, which is not NFC.
    let decomposed = "caf\u{65}\u{301}";
    let message = "Start\n";
    let stream = format!(
        "blob\n\
         mark :1\n\
         original-oid 5626abf0f72e74f6dc7e0eec4b95c6c7e0f5b7b8\n\
         data 4\none\n\n\
         commit refs/heads/{decomposed}\n\
         mark :2\n\
         original-oid 980aeeabdf5024a43392620b11f1d14de03a0bb5\n\
         author Ada <ada@example.com> 1767348245 -0700\n\
         committer Ada <ada@example.com> 1767348245 -0700\n\
         data {}\n{message}\
         M 100644 :1 a.txt\n\n",
        message.len(),
    )
    .into_bytes();

    let into = folder("unnameable-ref");
    let report = import::from_stream(std::io::Cursor::new(stream), &into)
        .expect("a ref historica cannot name does not stop a conversion");

    assert_eq!(report.revisions, 1, "the commit itself still crossed");
    assert!(
        report.bookmarks.is_empty(),
        "the ref should not have become a bookmark: {:?}",
        report.bookmarks
    );
    let said = report.uncarried.join("\n");
    assert!(
        said.contains(decomposed) && said.contains("did not cross"),
        "the report should name the ref and say it did not cross:\n{said}"
    );
}
