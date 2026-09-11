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
    let said = written.uncarried.join("\n");
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
/// by the next write; a branch git deleted is reported by the next import.
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

    // The store drops `topic`. Historica's API has no way to remove a bookmark,
    // so this test removes the file itself, which is what a person would do.
    fs::remove_file(store.join(STORE_DIR).join("names/topic.txt")).expect("the bookmark file");
    let written = export::to_repository(&store, &repo).expect("the write");
    assert_eq!(
        written.deleted,
        vec!["refs/heads/topic".to_owned()],
        "{written:?}"
    );
    assert!(!references(&repo).contains_key("refs/heads/topic"));

    // Git drops `scratch`. The import cannot remove the bookmark, and says so
    // rather than recreating the branch or staying quiet.
    run(&repo, &["branch", "-D", "scratch"]);
    let imported = import::from_repository(&repo, &store).expect("the import");
    let said = imported.uncarried.join("\n");
    assert!(
        said.contains("`scratch` was deleted in git"),
        "the import should say the branch went:\n{said}"
    );
    let written = export::to_repository(&store, &repo).expect("the write after");
    assert!(
        !references(&repo).contains_key("refs/heads/scratch"),
        "a write must not resurrect a branch git deleted: {written:?}"
    );
    assert!(
        written
            .uncarried
            .join("\n")
            .contains("`refs/heads/scratch` was not moved"),
        "{written:?}"
    );
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
            .uncarried
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
    assert!(
        report
            .uncarried
            .iter()
            .any(|line| line.contains("refs/t3/checkpoints/session/turn/")),
        "the ref is reported as somewhere else: {:?}",
        report.uncarried
    );

    let store = Store::open(into.join(STORE_DIR)).expect("the store opens");
    let heads = store.history().heads();
    assert_eq!(heads.len(), 1, "one root, one head: {heads:?}");
}
