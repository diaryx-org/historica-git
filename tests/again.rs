//! A conversion run again, onto what it made before — decision 0007.
//!
//! Every test here needs git, since what is being tested is agreement with the
//! repository's own record of earlier conversions, and that record is kept
//! under `.git/`. Each builds its own small repository so that the commits do
//! not depend on when or where the test ran.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use historica::core::RevisionId;
use historica::record::{self, Platform, Recording, Restriction};
use historica::store::{Name, STORE_DIR, Store};
use historica::working::Working;
use historica::wrote::Statement;
use historica_git::{export, import};

fn folder(name: &str) -> PathBuf {
    let at = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&at);
    at
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

/// Run git in `at`, with one person and one moment so that nothing depends on
/// the machine, and hand back what it printed.
fn run(at: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(at)
        .args(args)
        .env("GIT_AUTHOR_DATE", "2026-01-02T03:04:05-07:00")
        .env("GIT_COMMITTER_DATE", "2026-01-02T03:04:05-07:00")
        .env("GIT_AUTHOR_NAME", "Ada")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A repository of two commits on `main`.
fn build(at: &Path) {
    fs::create_dir_all(at).expect("a fresh directory");
    run(at, &["init", "--quiet", "--initial-branch=main", "."]);
    run(at, &["config", "commit.gpgsign", "false"]);
    run(at, &["config", "tag.gpgsign", "false"]);
    fs::write(at.join("a.txt"), "one\n").expect("a file");
    run(at, &["add", "-A"]);
    run(at, &["commit", "--quiet", "-m", "Start"]);
    fs::write(at.join("b.txt"), "two\n").expect("a file");
    run(at, &["add", "-A"]);
    run(at, &["commit", "--quiet", "-m", "Second"]);
}

/// Commit one more file in `at`, on whatever branch is checked out.
fn commit(at: &Path, name: &str, message: &str) -> String {
    fs::write(at.join(name), format!("{name}\n")).expect("a file");
    run(at, &["add", "-A"]);
    run(at, &["commit", "--quiet", "-m", message]);
    run(at, &["rev-parse", "HEAD"])
}

fn commits(at: &Path) -> Vec<String> {
    run(at, &["rev-list", "--all", "--reverse"])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn references(at: &Path) -> BTreeMap<String, String> {
    run(at, &["for-each-ref", "--format=%(refname) %(objectname)"])
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(name, oid)| (name.to_owned(), oid.to_owned()))
        .collect()
}

/// Record one more file in the store's folder, on the store's one head.
fn record(folder: &Path, name: &str, message: &str) -> RevisionId {
    let mut store = Store::open(folder.join(STORE_DIR)).expect("the store opens");
    let heads = store.history().heads();
    assert_eq!(heads.len(), 1, "the store should have one head: {heads:?}");
    fs::write(folder.join(name), format!("{name}\n")).expect("a file");
    let working = Working::read(folder, store.skipped()).expect("the folder reads");
    let recording = Recording {
        parents: heads.into_iter().collect(),
        author: "Ada <ada@example.com>".to_owned(),
        when: "2026-01-02T03:04:05-07:00".parse().expect("a timestamp"),
        message: message.to_owned(),
        moves: Vec::new(),
        at: Vec::new(),
        accepted: Default::default(),
        only: Restriction::Everything,
        extensions: BTreeMap::new(),
        kinds: Default::default(),
    };
    record::record(&mut store, &working, &recording, &mut Platform)
        .expect("the revision records")
        .revision
}

fn main_points_at(folder: &Path) -> RevisionId {
    let store = Store::open(folder.join(STORE_DIR)).expect("the store opens");
    let Some(Name::Change(change)) = store.name("main") else {
        panic!("`main` should be a bookmark on a change");
    };
    match store.history().change_state(&change) {
        historica::core::ChangeState::Resolved(revision) => revision.id,
        other => panic!("`main` should resolve: {other:?}"),
    }
}

fn record_files(at: &Path) -> (String, String) {
    let dir = at.join(".git/historica");
    let read = |name: &str| fs::read_to_string(dir.join(name)).unwrap_or_default();
    (read("commits"), read("refs"))
}

/// A second import takes what the repository gained and touches nothing else:
/// not the folder, which is somebody's working copy by then, and not the
/// revisions already held.
#[test]
fn importing_again_takes_what_the_repository_gained() {
    let Some(_) = git() else { return };
    let origin = folder("again-import-origin");
    build(&origin);
    let store = folder("again-import-store");
    let first = import::from_repository(&origin, &store).expect("the first import");
    assert!(!first.onto);
    assert_eq!(first.revisions, 2);

    // Meanwhile, in git: one more commit, a branch, and a tag on the old tip.
    let third = commit(&origin, "c.txt", "Third");
    run(&origin, &["branch", "topic"]);
    run(&origin, &["tag", "v1", "HEAD~1"]);
    // And in the folder: a file nobody has recorded, which an import must not
    // touch and must not record.
    fs::write(store.join("mine.txt"), "unrecorded\n").expect("a file");

    let again = import::from_repository(&origin, &store).expect("the second import");
    assert!(again.onto, "the store was there already");
    assert_eq!(
        again.revisions, 1,
        "only the new commit is recorded: {again:?}"
    );
    assert_eq!(
        again.held, 0,
        "the held tip was excluded, not re-read: {again:?}"
    );
    assert_eq!(again.moved, vec!["main".to_owned()], "{again:?}");
    assert_eq!(again.bookmarks, vec!["topic".to_owned(), "v1".to_owned()]);

    assert!(
        store.join("mine.txt").exists(),
        "the folder is not the import's"
    );
    assert!(
        !store.join("c.txt").exists(),
        "a second import writes into history/ and leaves the folder alone"
    );
    let opened = Store::open(store.join(STORE_DIR)).expect("the store opens");
    assert_eq!(opened.history().len(), 3);
    assert!(
        opened.name("mine.txt").is_none(),
        "nothing from the folder crossed into the store"
    );

    // The repository's record now names the commit and agrees about the refs.
    let (commits, refs) = record_files(&origin);
    assert!(
        commits.contains(&third),
        "the new commit is in the record:\n{commits}"
    );
    assert!(
        refs.contains(&format!("refs/heads/main {third}")),
        "the record says where main agreed:\n{refs}"
    );

    // And a third import, with nothing new, does nothing.
    let third = import::from_repository(&origin, &store).expect("the third import");
    assert_eq!((third.revisions, third.held), (0, 0), "{third:?}");
    assert!(
        third.moved.is_empty() && third.bookmarks.is_empty(),
        "{third:?}"
    );
}

/// A second write sends what the store gained and names the rest by object
/// ID, so the commits it wrote before are exactly the commits still there.
/// The repository's record is about the store an earlier conversion made, and
/// says nothing about a new one. An import into an empty folder, after the
/// repository was imported somewhere else, converts every commit rather than
/// recognising them as held by a store that is not there.
#[test]
fn importing_into_a_new_folder_ignores_what_another_store_holds() {
    let Some(_) = git() else { return };
    let origin = folder("again-elsewhere-origin");
    build(&origin);
    let first = folder("again-elsewhere-first");
    import::from_repository(&origin, &first).expect("the first import");

    let second = folder("again-elsewhere-second");
    let report = import::from_repository(&origin, &second).expect("the second import");
    assert_eq!(report.held, 0, "the new store held nothing");
    assert_eq!(
        report.revisions, 2,
        "every commit should have been converted"
    );
    assert_eq!(
        main_points_at(&second),
        main_points_at(&first),
        "two conversions of one repository are one history (decision 0003)"
    );
}

#[test]
fn writing_again_sends_only_what_the_store_gained() {
    let Some(_) = git() else { return };
    let origin = folder("again-write-origin");
    build(&origin);
    let store = folder("again-write-store");
    import::from_repository(&origin, &store).expect("the import");
    let repo = folder("again-write-repo");
    let first = export::to_repository(&store, &repo).expect("the first write");
    assert!(!first.onto);
    assert_eq!(commits(&repo), commits(&origin), "decision 0004's identity");

    let recorded = record(&store, "d.txt", "Fourth");
    let again = export::to_repository(&store, &repo).expect("the second write");
    assert!(again.onto);
    assert_eq!(again.commits, 1, "one revision was new: {again:?}");
    assert_eq!(again.reused, 2, "two were already there: {again:?}");
    assert_eq!(again.references, vec!["refs/heads/main".to_owned()]);
    let after = commits(&repo);
    assert_eq!(after.len(), 3);
    assert_eq!(
        &after[..2],
        &commits(&origin)[..],
        "earlier commits are untouched"
    );
    // HEAD was on main and the tree was clean, so it was brought up.
    assert_eq!(again.branch.as_deref(), Some("main"));
    assert!(
        repo.join("d.txt").exists(),
        "the working tree caught up: {again:?}\n{}",
        run(&repo, &["status"])
    );
    assert_eq!(run(&repo, &["status", "--porcelain"]), "");

    let (commits_file, refs_file) = record_files(&repo);
    assert!(
        commits_file.contains(&format!("{recorded} {}", after[2])),
        "the record names the revision and its commit:\n{commits_file}"
    );
    assert!(refs_file.contains(&format!("refs/heads/main {}", after[2])));

    let third = export::to_repository(&store, &repo).expect("the third write");
    assert_eq!((third.commits, third.reused), (0, 3), "{third:?}");
    assert!(third.references.is_empty(), "{third:?}");
}

/// A commit somebody makes in the written repository comes back standing on
/// the revision its parent was written from, and a write after that has
/// nothing to send.
#[test]
fn a_commit_made_in_git_comes_back_standing_on_its_revision() {
    let Some(_) = git() else { return };
    let origin = folder("again-round-origin");
    build(&origin);
    let store = folder("again-round-store");
    import::from_repository(&origin, &store).expect("the import");
    let repo = folder("again-round-repo");
    export::to_repository(&store, &repo).expect("the write");
    let before = main_points_at(&store);

    let fifth = commit(&repo, "e.txt", "Fifth");
    let back = import::from_repository(&repo, &store).expect("the import back");
    assert_eq!(back.revisions, 1, "{back:?}");
    assert_eq!(back.moved, vec!["main".to_owned()], "{back:?}");
    let after = main_points_at(&store);
    let opened = Store::open(store.join(STORE_DIR)).expect("the store opens");
    let revision = opened.revision(&after).expect("the new revision");
    assert_eq!(
        revision.parents.iter().copied().collect::<Vec<_>>(),
        vec![before],
        "the git commit stands on the revision its parent was written from"
    );

    let written = export::to_repository(&store, &repo).expect("the write after");
    assert_eq!(
        written.commits, 0,
        "the import recorded which commit that revision is: {written:?}"
    );
    assert_eq!(
        references(&repo)["refs/heads/main"],
        fifth,
        "main stays where git put it"
    );
}

/// Both sides moved a branch since the last write: neither conversion moves
/// it, and each says so.
#[test]
fn a_ref_both_sides_moved_is_held_back_and_said() {
    let Some(_) = git() else { return };
    let origin = folder("again-conflict-origin");
    build(&origin);
    let store = folder("again-conflict-store");
    import::from_repository(&origin, &store).expect("the import");
    let repo = folder("again-conflict-repo");
    export::to_repository(&store, &repo).expect("the write");

    let in_git = commit(&repo, "e.txt", "In git");
    record(&store, "f.txt", "In the store");
    let store_main = main_points_at(&store);

    let imported = import::from_repository(&repo, &store).expect("the import");
    assert_eq!(
        imported.revisions, 1,
        "the commit itself still crosses: {imported:?}"
    );
    assert!(imported.moved.is_empty(), "{imported:?}");
    let said = imported.uncarried.join("\n");
    assert!(
        said.contains("`main` was not moved") && said.contains("both moved"),
        "the import should say why main stayed:\n{said}"
    );
    assert_eq!(
        main_points_at(&store),
        store_main,
        "the store's main is the store's"
    );

    let written = export::to_repository(&store, &repo).expect("the write");
    let said = written.held_back.join("\n");
    assert!(
        said.contains("`refs/heads/main` was not moved") && said.contains("git moved it"),
        "the write should say why main stayed:\n{said}"
    );
    assert_eq!(
        references(&repo)["refs/heads/main"],
        in_git,
        "git's main is git's"
    );
}

/// A branch this tool made, whose bookmark the store no longer has, is deleted
/// by the next write; a branch git deleted takes its bookmark with it on the
/// next import.
#[test]
fn a_branch_one_side_dropped_is_carried_or_said() {
    let Some(_) = git() else { return };
    let origin = folder("again-drop-origin");
    build(&origin);
    run(&origin, &["branch", "topic"]);
    run(&origin, &["branch", "scratch"]);
    let store = folder("again-drop-store");
    import::from_repository(&origin, &store).expect("the import");
    let repo = folder("again-drop-repo");
    export::to_repository(&store, &repo).expect("the write");
    assert!(references(&repo).contains_key("refs/heads/topic"));

    // The store drops `topic`, as `historica name --delete` would.
    Store::open(store.join(STORE_DIR))
        .expect("the store opens")
        .remove_name("topic")
        .expect("the bookmark goes");
    let written = export::to_repository(&store, &repo).expect("the write");
    assert_eq!(
        written.deleted,
        vec!["refs/heads/topic".to_owned()],
        "{written:?}"
    );
    assert!(!references(&repo).contains_key("refs/heads/topic"));

    // Git drops `scratch`. The store left the bookmark where the branch was,
    // so git is the side that changed, and the bookmark goes with it.
    run(&repo, &["branch", "-D", "scratch"]);
    let imported = import::from_repository(&repo, &store).expect("the import");
    assert_eq!(imported.removed, vec!["scratch".to_owned()], "{imported:?}");
    let opened = Store::open(store.join(STORE_DIR)).expect("the store opens");
    assert!(opened.bookmark("scratch").is_none(), "the bookmark went");
    drop(opened);
    let (_, refs) = record_files(&repo);
    assert!(!refs.contains("refs/heads/scratch"), "{refs}");

    // And it is done: the next import has nothing to say about it, and the
    // next write neither resurrects the branch nor holds anything back.
    let again = import::from_repository(&repo, &store).expect("the import again");
    assert!(again.removed.is_empty(), "{again:?}");
    assert!(!again.uncarried.join("\n").contains("scratch"), "{again:?}");
    let written = export::to_repository(&store, &repo).expect("the write after");
    assert!(
        !references(&repo).contains_key("refs/heads/scratch"),
        "a write must not resurrect a branch git deleted: {written:?}"
    );
    assert!(written.held_back.is_empty(), "{written:?}");
}

/// A repository this tool never wrote gets the commits and none of its own
/// refs overwritten.
#[test]
fn a_repository_this_tool_never_wrote_keeps_its_own_refs() {
    let Some(_) = git() else { return };
    let origin = folder("again-stranger-origin");
    build(&origin);
    let store = folder("again-stranger-store");
    import::from_repository(&origin, &store).expect("the import");
    record(&store, "d.txt", "Fourth");

    // A second, unrelated repository with its own `main`.
    let stranger = folder("again-stranger-repo");
    fs::create_dir_all(&stranger).expect("a directory");
    run(
        &stranger,
        &["init", "--quiet", "--initial-branch=main", "."],
    );
    run(&stranger, &["config", "commit.gpgsign", "false"]);
    let theirs = commit(&stranger, "z.txt", "Theirs");

    let written = export::to_repository(&store, &stranger).expect("the write");
    assert_eq!(written.commits, 3, "every revision was sent: {written:?}");
    assert_eq!(
        references(&stranger)["refs/heads/main"],
        theirs,
        "their main is theirs"
    );
    assert!(
        written
            .held_back
            .join("\n")
            .contains("`refs/heads/main` was not moved"),
        "{written:?}"
    );
    // The commits are there, reachable from the head this tool parks work at.
    assert_eq!(commits(&stranger).len(), 4);
}

/// A write deletes only a branch a write of this tool's made. A branch git had
/// before this tool ever wrote here, which an import merely found agreeing
/// with the store, is somebody's, and dropping the bookmark does not reach it.
#[test]
fn a_write_deletes_only_a_branch_it_made() {
    let Some(_) = git() else { return };
    let origin = folder("again-made-origin");
    build(&origin);
    run(&origin, &["branch", "keepme"]);
    let store = folder("again-made-store");
    import::from_repository(&origin, &store).expect("the import");

    fs::remove_file(store.join(STORE_DIR).join("names/keepme.txt")).expect("the bookmark file");
    let written = export::to_repository(&store, &origin).expect("the first write here");
    assert!(written.deleted.is_empty(), "{written:?}");
    assert!(
        references(&origin).contains_key("refs/heads/keepme"),
        "a branch this tool never made is not this tool's to delete"
    );
}

/// A branch git deleted stays deleted, however many writes follow: the record
/// keeps the line that holds it back rather than forgetting it after one run.
#[test]
fn a_branch_git_deleted_stays_deleted_across_writes() {
    let Some(_) = git() else { return };
    let origin = folder("again-stay-origin");
    build(&origin);
    run(&origin, &["branch", "scratch"]);
    let store = folder("again-stay-store");
    import::from_repository(&origin, &store).expect("the import");
    let repo = folder("again-stay-repo");
    export::to_repository(&store, &repo).expect("the write");
    run(&repo, &["branch", "-D", "scratch"]);

    for _ in 0..3 {
        let written = export::to_repository(&store, &repo).expect("a write after");
        assert!(
            !references(&repo).contains_key("refs/heads/scratch"),
            "a write resurrected a branch git deleted: {written:?}"
        );
    }
}

/// A new branch grown from a commit that changed nothing — which the store
/// folded into its parent and the record does not name — still imports, on
/// the parent's revision.
#[test]
fn a_branch_grown_from_an_excluded_empty_commit_imports() {
    let Some(_) = git() else { return };
    let origin = folder("again-empty-origin");
    build(&origin);
    run(
        &origin,
        &["commit", "--quiet", "--allow-empty", "-m", "Nothing"],
    );
    let empty = run(&origin, &["rev-parse", "HEAD"]);
    commit(&origin, "c.txt", "Third");
    let store = folder("again-empty-store");
    let first = import::from_repository(&origin, &store).expect("the import");
    assert_eq!((first.revisions, first.empty), (3, 1), "{first:?}");

    run(&origin, &["checkout", "--quiet", "-b", "feature", &empty]);
    commit(&origin, "d.txt", "On the empty one");
    let again = import::from_repository(&origin, &store).expect("the import after");
    assert_eq!(again.revisions, 1, "{again:?}");
    assert_eq!(again.bookmarks, vec!["feature".to_owned()], "{again:?}");
}

/// An annotated tag is reported once, not once per way of finding it.
#[test]
fn an_annotated_tag_is_reported_once() {
    let Some(_) = git() else { return };
    let origin = folder("again-annotated-origin");
    build(&origin);
    run(&origin, &["tag", "-a", "v1", "-m", "A tag with a message"]);
    let store = folder("again-annotated-store");
    let report = import::from_repository(&origin, &store).expect("the import");
    let said = report.uncarried.join("\n");
    assert!(
        said.contains("one annotated tag did not cross"),
        "one tag, said once:\n{said}"
    );
}

/// A repository under SHA-256 spells the null object ID sixty-four wide, and
/// a write onto one has to delete its staging ref in that spelling.
#[test]
fn a_sha256_repository_is_written_onto() {
    let Some(_) = git() else { return };
    let origin = folder("again-sha256-origin");
    fs::create_dir_all(&origin).expect("a directory");
    run(
        &origin,
        &[
            "init",
            "--quiet",
            "--object-format=sha256",
            "--initial-branch=main",
            ".",
        ],
    );
    run(&origin, &["config", "commit.gpgsign", "false"]);
    commit(&origin, "a.txt", "Start");
    let store = folder("again-sha256-store");
    import::from_repository(&origin, &store).expect("the import");

    let repo = folder("again-sha256-repo");
    fs::create_dir_all(&repo).expect("a directory");
    run(
        &repo,
        &[
            "init",
            "--quiet",
            "--object-format=sha256",
            "--initial-branch=main",
            ".",
        ],
    );
    let written = export::to_repository(&store, &repo).expect("the write");
    assert_eq!(written.commits, 1, "{written:?}");
    assert!(!references(&repo).contains_key("refs/historica/staging"));
}

/// Adopting an existing checkout replays in scratch space: staged, unstaged,
/// and untracked work all survive byte for byte.
#[test]
fn colocating_a_repository_preserves_its_working_tree() {
    let Some(_) = git() else { return };
    let at = folder("colocate-repository");
    build(&at);
    fs::write(at.join("a.txt"), "unstaged\n").expect("an edit");
    fs::write(at.join("b.txt"), "staged\n").expect("an edit");
    run(&at, &["add", "b.txt"]);
    fs::write(at.join("mine.txt"), "untracked\n").expect("an edit");
    let before = run(&at, &["status", "--porcelain"]);

    let report = import::from_repository(&at, &at).expect("the colocated import");
    assert_eq!(report.revisions, 2, "{report:?}");
    assert_eq!(fs::read_to_string(at.join("a.txt")).unwrap(), "unstaged\n");
    assert_eq!(fs::read_to_string(at.join("b.txt")).unwrap(), "staged\n");
    assert_eq!(
        fs::read_to_string(at.join("mine.txt")).unwrap(),
        "untracked\n"
    );
    assert_eq!(
        run(&at, &["status", "--porcelain"]),
        "M a.txt\n M b.txt\n?? mine.txt",
        "the bytes survive and the derived index is reset to HEAD; before was {before:?}"
    );
    let exclude = fs::read_to_string(at.join(".git/info/exclude")).expect("the exclude");
    assert_eq!(
        exclude.lines().filter(|line| *line == "/history/").count(),
        1
    );
    let store = Store::open(at.join(STORE_DIR)).expect("the store opens");
    assert!(
        store
            .skipped()
            .rules()
            .any(|rule| rule.to_string() == "private .git")
    );
    assert!(
        store
            .skipped()
            .rules()
            .any(|rule| rule.to_string() == "private .git/")
    );
}

/// A repository derived around a store claims only git's index. Files that
/// differ from the recorded revision remain as Historica working-copy edits.
#[test]
fn colocating_a_store_never_checks_out_files() {
    let Some(_) = git() else { return };
    let origin = folder("colocate-store-origin");
    build(&origin);
    let at = folder("colocate-store");
    import::from_repository(&origin, &at).expect("the import");
    fs::write(at.join("a.txt"), "mine\n").expect("an edit");

    let report = export::to_repository(&at, &at).expect("the colocated write");
    assert_eq!(fs::read_to_string(at.join("a.txt")).unwrap(), "mine\n");
    assert_eq!(report.branch.as_deref(), Some("main"), "{report:?}");
    assert!(run(&at, &["status", "--porcelain"]).contains("a.txt"));
}

/// Naming the colocated directory explicitly is the resolution: git's branch
/// wins even if its bookmark moved independently since the last agreement.
#[test]
fn explicit_colocated_import_gives_git_precedence() {
    let Some(_) = git() else { return };
    let at = folder("colocate-import-wins");
    build(&at);
    import::from_repository(&at, &at).expect("the colocated import");
    let git_commit = commit(&at, "git.txt", "In git");
    record(&at, "store.txt", "In the store");

    let report = import::from_repository(&at, &at).expect("the explicit import");
    assert_eq!(report.moved, vec!["main".to_owned()], "{report:?}");
    let (_, refs) = record_files(&at);
    assert!(
        refs.contains(&format!("refs/heads/main {git_commit}")),
        "{refs}"
    );
}

/// A ref that is neither a branch nor a tag is a fact about somewhere else
/// (decision 0006), and so is everything reachable only through it. An
/// editor that checkpoints under a ref directory of its own writes parentless
/// snapshot commits there, and each one `--all` brought across became a root:
/// a store with one root per checkpoint cannot record a merge, because every
/// path in it was placed once per root. Branches and tags are the history.
#[test]
fn a_ref_that_is_neither_branch_nor_tag_brings_no_commits() {
    let Some(_) = git() else { return };
    let at = folder("elsewhere-repository");
    build(&at);
    // A parentless commit of the same tree, under a tool's own ref directory.
    let tree = run(&at, &["write-tree"]);
    let checkpoint = run(&at, &["commit-tree", &tree, "-m", "checkpoint"]);
    run(
        &at,
        &[
            "update-ref",
            "refs/t3/checkpoints/session/turn/1",
            &checkpoint,
        ],
    );
    assert_eq!(commits(&at).len(), 3, "git holds the checkpoint too");

    let into = folder("elsewhere-store");
    let report = import::from_repository(&at, &into).expect("the import");
    assert_eq!(report.commits, 2, "the two on main, and not the checkpoint");
    assert_eq!(report.revisions, 2, "{report:?}");
    assert_eq!(report.bookmarks, vec!["main".to_owned()]);
    // Reported as somewhere else, and without claiming the checkpoint is in
    // the store, which it is not.
    let said = report.standing.join("\n");
    assert!(
        said.contains("refs/t3/checkpoints/session/turn/")
            && said.contains("what only they reach did not cross either")
            && !said.contains("every commit they named is in the store"),
        "the ref is reported as somewhere else: {said}"
    );

    let store = Store::open(into.join(STORE_DIR)).expect("the store opens");
    let heads = store.history().heads();
    assert_eq!(heads.len(), 1, "one root, one head: {heads:?}");
}

/// What `historica record --fields` would have said it wrote: the revision,
/// and `main`, which followed it.
fn wrote(revision: RevisionId) -> Statement {
    let mut statement = Statement::new();
    statement.revision(revision).name("main");
    statement
}

/// Decision 0009: a statement with no lines is the cheap case, and it is
/// cheap because nothing is opened — not even a folder that is not there.
#[test]
fn the_run_after_a_command_that_wrote_nothing_opens_nothing() {
    let nowhere = folder("run-nowhere");
    let done = export::after(&Statement::new(), &nowhere, &nowhere).expect("nothing to do");
    assert!(done.is_none());
    assert!(!nowhere.exists(), "nothing was made");
}

/// Decision 0009: the run brings a colocated repository up to what the
/// command recorded, and leaves git's status clean.
#[test]
fn the_run_keeps_a_colocated_repository_current() {
    let Some(_) = git() else { return };
    let at = folder("run-current");
    build(&at);
    import::from_repository(&at, &at).expect("the colocated import");

    let recorded = record(&at, "c.txt", "Third");
    let report = export::after(&wrote(recorded), &at, &at)
        .expect("the run")
        .expect("something was written");
    assert_eq!(report.commits, 1, "{report:?}");
    assert_eq!(report.references, vec!["refs/heads/main".to_owned()]);
    assert!(report.held_back.is_empty(), "{report:?}");
    let (commits_file, _) = record_files(&at);
    let head = run(&at, &["rev-parse", "HEAD"]);
    assert!(
        commits_file.contains(&format!("{recorded} {head}")),
        "HEAD is the commit the revision became:\n{commits_file}"
    );
    assert_eq!(
        run(&at, &["status", "--porcelain"]),
        "",
        "the index followed"
    );
}

/// Decision 0009 under 0008's detection: a branch git moved on its own is
/// said in the same breath and not followed, the rest of the run is done, and
/// the index is left alone because HEAD no longer names the folder's revision.
#[test]
fn the_run_says_a_ref_git_moved_and_does_not_follow_it() {
    let Some(_) = git() else { return };
    let at = folder("run-held-back");
    build(&at);
    import::from_repository(&at, &at).expect("the colocated import");
    let in_git = commit(&at, "git.txt", "In git");

    let recorded = record(&at, "store.txt", "In the store");
    let report = export::after(&wrote(recorded), &at, &at)
        .expect("a ref held back is a report, not an error")
        .expect("something was written");
    assert_eq!(report.commits, 1, "the revision still crossed: {report:?}");
    assert!(
        report
            .held_back
            .join("\n")
            .contains("`refs/heads/main` was not moved"),
        "{report:?}"
    );
    assert!(report.branch.is_none(), "{report:?}");
    assert_eq!(
        references(&at)["refs/heads/main"],
        in_git,
        "git's main is git's"
    );
}

/// Decision 0009: a statement naming a revision this store does not hold is
/// about another store, and is refused before anything is written.
#[test]
fn the_run_refuses_a_statement_about_another_store() {
    let Some(_) = git() else { return };
    let elsewhere = folder("run-elsewhere-other");
    build(&elsewhere);
    import::from_repository(&elsewhere, &elsewhere).expect("the other store");
    let theirs = record(&elsewhere, "theirs.txt", "Theirs");

    let at = folder("run-elsewhere");
    build(&at);
    import::from_repository(&at, &at).expect("the colocated import");
    record(&at, "mine.txt", "Mine");
    let before = references(&at);

    let refused = export::after(&wrote(theirs), &at, &at);
    assert!(
        matches!(refused, Err(export::Error::Elsewhere { revision }) if revision == theirs),
        "{refused:?}"
    );
    assert_eq!(references(&at), before, "nothing was written to git");
}

/// Decision 0009: keeping a repository current never makes one.
#[test]
fn the_run_never_makes_a_repository() {
    let Some(_) = git() else { return };
    let origin = folder("run-unprojected-origin");
    build(&origin);
    let at = folder("run-unprojected");
    import::from_repository(&origin, &at).expect("the import");
    let recorded = record(&at, "c.txt", "Third");

    let refused = export::after(&wrote(recorded), &at, &at);
    assert!(
        matches!(refused, Err(export::Error::Unprojected { .. })),
        "{refused:?}"
    );
    assert!(!at.join(".git").exists(), "no repository was made");
}

/// A colocated store with no branch leaves HEAD on git's own default, unborn;
/// once a bookmark names one, the run points HEAD at it, so git's status is
/// not a folder of untracked files.
#[test]
fn the_run_points_an_unborn_head_at_the_branch_it_wrote() {
    let Some(_) = git() else { return };
    let origin = folder("run-unborn-origin");
    build(&origin);
    let at = folder("run-unborn");
    import::from_repository(&origin, &at).expect("the import");
    let main = {
        let mut store = Store::open(at.join(STORE_DIR)).expect("the store opens");
        let main = store.name("main").expect("main is a bookmark");
        assert!(store.remove_name("main").expect("the bookmark goes"));
        main
    };
    export::to_repository(&at, &at).expect("the colocated write");
    run(&at, &["symbolic-ref", "HEAD", "refs/heads/trunk"]);

    let mut store = Store::open(at.join(STORE_DIR)).expect("the store opens");
    store.set_name("main", main).expect("the bookmark returns");
    drop(store);
    let mut statement = Statement::new();
    statement.name("main");
    let report = export::after(&statement, &at, &at)
        .expect("the run")
        .expect("something was written");
    assert_eq!(report.branch.as_deref(), Some("main"), "{report:?}");
    assert_eq!(run(&at, &["symbolic-ref", "HEAD"]), "refs/heads/main");
    assert_eq!(run(&at, &["status", "--porcelain"]), "");
}

/// `git switch -c` makes a branch the store has no bookmark for. What
/// historica records then goes to a bookmark that is not HEAD's, and the
/// index stays with HEAD; the run says so rather than leaving `git status`
/// to show the recorded work as untracked without a word.
#[test]
fn the_run_says_head_is_on_a_branch_git_made() {
    let Some(_) = git() else { return };
    let at = folder("run-git-branch");
    build(&at);
    import::from_repository(&at, &at).expect("the colocated import");
    run(&at, &["switch", "--quiet", "-c", "feature"]);

    let recorded = record(&at, "c.txt", "On the store's main");
    let report = export::after(&wrote(recorded), &at, &at)
        .expect("the run")
        .expect("something was written");
    assert_eq!(report.references, vec!["refs/heads/main".to_owned()]);
    let said = report.held_back.join("\n");
    assert!(
        said.contains("HEAD is on `feature`")
            && said.contains("went to `main`")
            && said.contains("`import`"),
        "{said}"
    );
    assert_eq!(run(&at, &["symbolic-ref", "HEAD"]), "refs/heads/feature");

    // The remedy it gives touches no file and leaves git agreeing.
    run(&at, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    run(&at, &["reset", "--quiet"]);
    assert_eq!(run(&at, &["status", "--porcelain"]), "");
}

/// After `git fetch`, a remote-tracking ref names work no branch has reached,
/// and the import says so by name rather than claiming every commit it named
/// is in the store. Once a branch reaches it, it is one of the refs that are
/// always somewhere else, said by a whole conversion and not by every run
/// onto it — decision 0010.
#[test]
fn a_remote_tracking_ref_ahead_of_every_branch_is_said() {
    let Some(_) = git() else { return };
    let origin = folder("remote-ahead-origin");
    build(&origin);
    let at = folder("remote-ahead");
    run(
        Path::new(env!("CARGO_TARGET_TMPDIR")),
        &[
            "clone",
            "--quiet",
            origin.to_str().expect("a UTF-8 path"),
            "remote-ahead",
        ],
    );
    let first = import::from_repository(&at, &at).expect("the colocated import");
    assert!(
        first.standing.join("\n").contains("refs/remotes/origin/"),
        "{first:?}"
    );

    commit(&origin, "c.txt", "Upstream");
    run(&at, &["fetch", "--quiet"]);
    let fetched = import::from_repository(&at, &at).expect("the import after a fetch");
    assert_eq!(fetched.revisions, 0, "{fetched:?}");
    let said = fetched.uncarried.join("\n");
    assert!(
        said.contains("`refs/remotes/origin/main` names work the store does not hold"),
        "{said}"
    );
    assert!(!fetched.behind, "nothing arrived: {fetched:?}");

    run(&at, &["merge", "--quiet", "--ff-only", "origin/main"]);
    let merged = import::from_repository(&at, &at).expect("the import after a merge");
    assert_eq!(merged.revisions, 1, "{merged:?}");
    assert!(merged.uncarried.is_empty(), "{merged:?}");
    assert!(
        merged
            .standing
            .join("\n")
            .contains("every commit they named is in the store"),
        "{merged:?}"
    );
    // Git already wrote the files, so the folder holds what arrived.
    assert!(!merged.behind, "{merged:?}");
}

/// A separate folder does not hold what a second import recorded, and the
/// report says `historica update` is what brings it; a run that recorded
/// nothing says nothing of the kind.
#[test]
fn a_separate_folder_is_behind_only_when_something_arrived() {
    let Some(_) = git() else { return };
    let origin = folder("behind-origin");
    build(&origin);
    let store = folder("behind-store");
    import::from_repository(&origin, &store).expect("the import");

    let idle = import::from_repository(&origin, &store).expect("an import of nothing");
    assert!(!idle.behind, "{idle:?}");
    commit(&origin, "c.txt", "Third");
    let again = import::from_repository(&origin, &store).expect("the second import");
    assert!(again.behind, "{again:?}");
}

/// Decision 0011: a colocated store follows the branch HEAD is on, the
/// bookmarks it already has, and the tags those reach. A branch beside it — a
/// worktree's, an agent's — stays in git and is named, so that the store's one
/// head is the line the folder holds.
#[test]
fn a_colocated_store_follows_the_branch_head_is_on() {
    let Some(_) = git() else { return };
    let at = folder("follows-head");
    build(&at);
    run(&at, &["tag", "v1"]);
    run(&at, &["switch", "--quiet", "-c", "feature"]);
    commit(&at, "c.txt", "Ahead of main");
    run(&at, &["tag", "v-feature"]);
    run(&at, &["switch", "--quiet", "main"]);
    fs::remove_file(at.join("c.txt")).ok();

    let report = import::from_repository(&at, &at).expect("the colocated import");
    assert_eq!(report.bookmarks, ["main", "v1"], "{report:?}");
    let said = report.uncarried.join("\n");
    assert!(
        said.contains(
            "`refs/heads/feature`, `refs/tags/v-feature` name work the store does not hold"
        ),
        "{said}"
    );
    let store = Store::open(at.join(STORE_DIR)).expect("the store opens");
    let heads: Vec<RevisionId> = store.history().heads().into_iter().collect();
    assert_eq!(heads, [main_points_at(&at)]);
    drop(store);

    // An import with HEAD on the branch brings it in, and from then on the
    // store follows it as a bookmark it has.
    run(&at, &["switch", "--quiet", "feature"]);
    let switched = import::from_repository(&at, &at).expect("the import on feature");
    assert_eq!(switched.bookmarks, ["feature", "v-feature"], "{switched:?}");
    assert!(switched.uncarried.is_empty(), "{switched:?}");
}

/// Decision 0011: what git ignores becomes a skip rule where historica's rules
/// can say it exactly, and is said where they cannot. A rule that covered a
/// tracked file would record that file's deletion, so it is not made.
#[test]
fn a_colocated_store_skips_what_git_ignores() {
    let Some(_) = git() else { return };
    let at = folder("skips-ignored");
    build(&at);
    run(&at, &["config", "core.excludesFile", "/dev/null"]);
    fs::write(at.join(".gitignore"), "/target\n*.log\nfile?.tmp\n").expect("a file");
    fs::write(at.join("kept.log"), "kept\n").expect("a file");
    run(&at, &["add", "-f", ".gitignore", "kept.log"]);
    run(&at, &["commit", "--quiet", "-m", "Ignore"]);
    fs::create_dir_all(at.join("target")).expect("a directory");
    fs::write(at.join("target/build"), "out\n").expect("a file");

    let report = import::from_repository(&at, &at).expect("the colocated import");
    assert_eq!(
        report.skipped,
        ["skip target", "skip target/"],
        "{report:?}"
    );
    let standing = report.standing.join("\n");
    assert!(
        standing.contains("2 of git's ignore lines")
            && standing.contains(".gitignore:2, because it covers a file git tracks")
            && standing.contains(".gitignore:3"),
        "{standing}"
    );
    let store = Store::open(at.join(STORE_DIR)).expect("the store opens");
    assert!(store.skipped().skips("target/build"));
    assert!(!store.skipped().skips("kept.log"));
    let working = format!(
        "{:?}",
        Working::read(&at, store.skipped()).expect("the folder reads")
    );
    assert!(working.contains("kept.log"), "{working}");
    assert!(!working.contains("target/build"), "{working}");
    drop(store);

    // Said once: a run onto it adds nothing and names nothing.
    let again = import::from_repository(&at, &at).expect("the import again");
    assert!(again.skipped.is_empty(), "{again:?}");
}

/// Commit everything in `at` as it stands now.
fn commit_all(at: &Path, message: &str) {
    run(at, &["add", "-A"]);
    run(at, &["commit", "--quiet", "-m", message]);
}

/// A repository whose one commit holds the file `p`.
fn a_file_at_p(name: &str) -> PathBuf {
    let origin = folder(name);
    fs::create_dir_all(&origin).expect("a fresh directory");
    run(&origin, &["init", "--quiet", "--initial-branch=main", "."]);
    run(&origin, &["config", "commit.gpgsign", "false"]);
    fs::write(origin.join("p"), "a file\n").expect("a file");
    commit_all(&origin, "A file");
    origin
}

/// Put `p/q` where the file `p` was, which is the same change however the file
/// left.
fn a_folder_at_p(origin: &Path) {
    fs::create_dir_all(origin.join("p")).expect("a folder");
    fs::write(origin.join("p/q"), "in the folder\n").expect("a file");
}

/// Import `origin` and write it out again, holding the two to the identity the
/// README states, and hand back the store's folder.
fn comes_back_the_same(origin: &Path, name: &str) -> PathBuf {
    let store = folder(&format!("{name}-store"));
    import::from_repository(origin, &store).expect("the import");
    let repo = folder(&format!("{name}-repo"));
    export::to_repository(&store, &repo).expect("the write");
    assert_eq!(commits(&repo), commits(origin), "decision 0004's identity");
    store
}

/// `git fast-export` writes what the commit put in the folder before it deletes
/// the file, and the delete names only the file.
#[test]
fn a_file_replaced_by_a_folder_comes_back() {
    let Some(_) = git() else { return };
    let origin = a_file_at_p("folder-over-file-origin");
    fs::remove_file(origin.join("p")).expect("the file goes");
    a_folder_at_p(&origin);
    commit_all(&origin, "A folder");

    let store = comes_back_the_same(&origin, "folder-over-file");
    assert_eq!(
        fs::read_to_string(store.join("p/q")).expect("the folder crossed"),
        "in the folder\n"
    );
}

/// The same stream with `R p old` where the `D p` was: the rename carries the
/// file, and only the file.
#[test]
fn a_file_moved_aside_for_a_folder_comes_back() {
    let Some(_) = git() else { return };
    let origin = a_file_at_p("folder-after-move-origin");
    run(&origin, &["mv", "p", "old"]);
    a_folder_at_p(&origin);
    commit_all(&origin, "A folder");

    let store = comes_back_the_same(&origin, "folder-after-move");
    assert_eq!(
        fs::read_to_string(store.join("old")).expect("the file moved"),
        "a file\n"
    );
    assert_eq!(
        fs::read_to_string(store.join("p/q")).expect("the folder crossed"),
        "in the folder\n"
    );
}

/// `R p p/q`, after an `M p/r` this commit wrote: the file goes beneath the path
/// it left, and what is already there stays.
#[test]
fn a_file_moved_into_a_folder_of_its_own_name_comes_back() {
    let Some(_) = git() else { return };
    let origin = a_file_at_p("file-into-own-folder-origin");
    run(&origin, &["mv", "p", "held"]);
    fs::create_dir_all(origin.join("p")).expect("a folder");
    run(&origin, &["mv", "held", "p/q"]);
    fs::write(origin.join("p/r"), "beside it\n").expect("a file");
    commit_all(&origin, "Into the folder");

    let store = comes_back_the_same(&origin, "file-into-own-folder");
    assert_eq!(
        fs::read_to_string(store.join("p/q")).expect("the file moved"),
        "a file\n"
    );
    assert_eq!(
        fs::read_to_string(store.join("p/r")).expect("the folder crossed"),
        "beside it\n"
    );
}

/// A rename out of a path that a new file then takes: the rename has to read
/// the old file and the path has to end up with the new one.
#[test]
fn a_file_moved_and_written_anew_comes_back() {
    let Some(_) = git() else { return };
    let origin = a_file_at_p("moved-and-written-anew-origin");
    run(&origin, &["mv", "p", "old"]);
    fs::write(origin.join("p"), "another file\n").expect("a file");
    commit_all(&origin, "Moved and written anew");

    let store = comes_back_the_same(&origin, "moved-and-written-anew");
    assert_eq!(
        fs::read_to_string(store.join("old")).expect("the file moved"),
        "a file\n"
    );
    assert_eq!(
        fs::read_to_string(store.join("p")).expect("the new file"),
        "another file\n"
    );
}
