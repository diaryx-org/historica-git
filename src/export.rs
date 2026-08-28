//! A Historica store, written out as a git repository.
//!
//! The other direction of decision 0002's bridge: [`crate::stream::Writer`]
//! spells a fast-import stream and `git fast-import` reads it. No git object is
//! written here either.
//!
//! Decision 0004 is what this module is for. A commit is a function of the
//! revision it came from — the tree, the parents, the author line, the message,
//! and a committer that is the author restated rather than read from the clock
//! — so converting one store twice, or on two machines, produces the same
//! commits, and the commit-to-revision correspondence needs filing nowhere for
//! the sake of correctness. Decision 0007 files it anyway, in the repository,
//! for the sake of not sending a commit git already has: a revision the
//! repository remembers is named by object ID rather than written again.
//!
//! What historica holds and git does not — supersession, a contested file, a
//! forgotten payload — is reported or refused, never approximated, which is
//! decision 0001's rule read from this end.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command as Process, Stdio};

use historica::core::{ChangeState, RevisionId};
use historica::store::{Content, STORE_DIR, Store};
use historica::tree::{Kind, Tree, TreeContest};

use crate::carried;
use crate::plumbing::{Pointed, Repository};
use crate::remembered::{Commits, DIRECTORY, Left, Refs};
use crate::stream::{Blob, Change, Command, Commit, Content as Carried, DataRef, Mark, Mode};
use crate::stream::{Person, Reset, Signature, Writer};

mod error;

pub use error::Error;

/// The ref every commit is written to on its way in.
///
/// A `commit` command has to name a ref, and the ref a revision *belongs* to is
/// only known once every bookmark has been read. So the stream parks each
/// commit here, moves the real refs at the end, and deletes this one — which is
/// why it is under `refs/historica/` rather than anywhere a person looks.
const STAGING: &str = "refs/historica/staging";

/// Where a head no bookmark names is left, so that nothing written is
/// unreachable.
const UNNAMED: &str = "refs/historica/heads";

/// Where a bookmark on a change goes: a branch, which moves as the work does.
const HEADS: &str = "refs/heads/";

/// Where a bookmark on a revision goes: a tag, which is pinned and does not.
const TAGS: &str = "refs/tags/";

/// What a conversion did, and what it could not carry.
///
/// The counterpart of [`crate::import::Report`], and decision 0001 asks the
/// same of it: each side holds facts the other has no place for, and dropping
/// them quietly is the failure that matters.
#[derive(Clone, Default, Debug)]
pub struct Report {
    /// Revisions read from the store.
    pub revisions: usize,
    /// Commits written to the stream.
    pub commits: usize,
    /// Commits the repository already held, named rather than written again.
    pub reused: usize,
    /// Refs the conversion moved.
    pub references: Vec<String>,
    /// Refs the conversion deleted, because it made them and the store no
    /// longer names them.
    pub deleted: Vec<String>,
    /// The branch the conversion would have a fresh repository check out.
    pub branch: Option<String>,
    /// Whether the repository existed before this conversion.
    pub onto: bool,
    /// One line per kind of fact that did not cross, for a person to read.
    pub uncarried: Vec<String>,
}

impl Report {
    fn note(&mut self, line: String) {
        self.uncarried.push(line);
    }
}

/// Convert the store under `folder` into the git repository at `repository`.
///
/// An empty or absent target gets `git init` and a whole conversion. A target
/// holding a repository gets what the store has gained since it was last
/// written — or, for a repository this tool never wrote, every commit and no
/// ref that git already has pointing elsewhere. Anything else is refused, for
/// the reason import's target must be free: a conversion only writes where it
/// can be sure it owns everything it touches.
pub fn to_repository(folder: &Path, repository: &Path) -> Result<Report, Error> {
    let (repository, onto) = match Repository::at(repository) {
        Some(existing) => (existing, true),
        None => {
            free(repository)?;
            std::fs::create_dir_all(repository).map_err(|error| Error::io(repository, error))?;
            let started = Process::new("git")
                .arg("-C")
                .arg(repository)
                .args(["init", "--quiet"])
                .status()
                .map_err(Error::Spawn)?;
            if !started.success() {
                return Err(Error::GitFailed {
                    status: started.code(),
                    said: vec!["`git init` refused the target directory".to_owned()],
                });
            }
            let made = Repository::at(repository).ok_or_else(|| Error::NotFree {
                at: repository.to_path_buf(),
                because: "`git init` ran and left no `.git` here".to_owned(),
            })?;
            (made, false)
        }
    };

    let git_dir = repository.git_dir()?;
    let mut commits = Commits::read(&git_dir)?;
    // A remembered commit the repository no longer holds — somebody ran `gc`
    // with nothing pointing at it — is forgotten, and the revision is sent
    // again, which decision 0004 makes the same commit.
    let holding = repository.holding(commits.oids())?;
    for gone in commits
        .oids()
        .filter(|oid| !holding.contains(*oid))
        .map(str::to_owned)
        .collect::<Vec<_>>()
    {
        commits.remove_oid(&gone);
    }
    let known = Known {
        commits,
        refs: Refs::read(&git_dir)?,
        current: repository.refs()?,
        null: repository.null_oid()?,
    };
    // Asked now, against the HEAD the person last checked out. Once the refs
    // have moved, an untouched index reads as a working tree full of changes
    // against the new one, which is the opposite of what the question means.
    let clean = onto && repository.is_clean()?;

    let marks_file = git_dir.join(DIRECTORY).join("marks.tmp");
    std::fs::create_dir_all(git_dir.join(DIRECTORY))
        .map_err(|error| Error::io(&git_dir.join(DIRECTORY), error))?;
    let mut child = Process::new("git")
        .arg("-C")
        .arg(repository.path())
        .args(["fast-import", "--quiet", "--force"])
        .arg(format!("--export-marks={}", marks_file.display()))
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Error::Spawn)?;

    // Drained on its own thread, for the reason import drains git's: a pipe
    // nobody reads fills, and then both sides wait forever.
    let complaints = child.stderr.take().expect("stderr was piped");
    let listening = std::thread::spawn(move || {
        BufReader::new(complaints)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
    });

    let stream = child.stdin.take().expect("stdin was piped");
    let outcome = convert(folder, stream, &known);

    let said = listening.join().unwrap_or_default();
    let status = child.wait().map_err(Error::Spawn)?;
    let mut written = match outcome {
        Ok(written) => written,
        Err(error) => {
            let _ = std::fs::remove_file(&marks_file);
            return Err(error);
        }
    };
    if !status.success() {
        let _ = std::fs::remove_file(&marks_file);
        return Err(Error::GitFailed {
            status: status.code(),
            said,
        });
    }

    // What git made of what was sent. `--export-marks` is how object IDs are
    // learned, as decision 0004 said it would be.
    let marks = read_marks(&marks_file);
    let _ = std::fs::remove_file(&marks_file);
    let marks = marks?;
    let mut commits = known.commits;
    for (revision, mark) in &written.sent {
        if let Some(oid) = marks.get(mark) {
            commits.insert(*revision, oid.clone());
        }
    }
    commits.write(&git_dir)?;

    // The record carried forward, not rewritten: a ref held back keeps the
    // line that held it back, or the next write would forget why and undo a
    // deletion somebody meant. What this write moved is marked as made; what
    // it merely found in agreement keeps whatever it was.
    let mut left = known.refs.to_map();
    for (reference, target) in &written.ours {
        let oid = match target {
            DataRef::Oid(oid) => Some(oid.clone()),
            DataRef::Mark(mark) => marks.get(mark).cloned(),
        };
        if let Some(oid) = oid {
            let made = written.moved.contains(reference) || known.refs.made(reference);
            left.insert(reference.clone(), Left { oid, made });
        }
    }
    for reference in &written.deleted {
        left.remove(reference);
    }
    Refs::write(&git_dir, &left)?;

    let mut report = std::mem::take(&mut written.report);
    report.onto = onto;
    if onto {
        // An existing repository has a HEAD of its own; `branch` is only what
        // the working tree was actually brought up to.
        report.branch = None;
        catch_up(&repository, &written, clean, &mut report)?;
    } else {
        check_out(&repository, &mut report)?;
    }
    Ok(report)
}

/// Point HEAD at a branch and put the files in the folder.
///
/// Nothing in the store says which branch was checked out — historica has no
/// HEAD, because the folder *is* the tree and `update` is what moves it — so
/// the conversion chooses the first branch a bookmark names and says which. A
/// store with no branch in it leaves git's own default alone, and the
/// repository is one where every ref is a tag or nothing.
///
/// Two commands rather than more stream. `fast-import` moves refs and cannot
/// make HEAD a *symbolic* one, and it does not touch the index or the working
/// tree at all — so a repository left as the import found it would show every
/// file as deleted, which is an alarming way to hand somebody a conversion. Both
/// are git writing to its own repository, which is decision 0002's arrangement
/// rather than an exception to it: still no git object written here.
fn check_out(repository: &Repository, report: &mut Report) -> Result<(), Error> {
    let Some(branch) = report.branch.clone() else {
        report.note(
            "nothing says which branch is checked out, because no bookmark named a \
             branch; git's own default is what HEAD points at and it points at \
             nothing"
                .to_owned(),
        );
        return Ok(());
    };
    repository.run(&["symbolic-ref", "HEAD", &format!("{HEADS}{branch}")])?;
    repository.run(&["reset", "--quiet", "--hard"])?;
    Ok(())
}

/// Bring an existing repository's working tree up to the branch HEAD names,
/// where that branch moved and there is nothing of anybody's in the way.
///
/// The target was somebody's repository before this ran, so the rule is
/// import's: only remove what this tool can be sure it owns. A clean index and
/// working tree are exactly that — `reset --hard` leaves untracked files alone
/// — and anything else is left as it is and said.
fn catch_up(
    repository: &Repository,
    written: &Written,
    clean: bool,
    report: &mut Report,
) -> Result<(), Error> {
    let Some(head) = repository.head()? else {
        return Ok(());
    };
    let branch = head.strip_prefix(HEADS).unwrap_or(&head).to_owned();
    if written.deleted.contains(&head) {
        report.note(format!(
            "HEAD is on `{branch}`, which this conversion deleted; git will treat \
             the next commit as the first on a branch of that name"
        ));
        return Ok(());
    }
    if !written.moved.contains(&head) {
        return Ok(());
    }
    if clean {
        repository.run(&["reset", "--quiet", "--hard"])?;
        report.branch = Some(branch);
    } else {
        report.note(format!(
            "HEAD is on `{branch}`, which moved, and the working tree has changes of \
             its own; it was left as it is, and `git reset --hard` is what brings \
             it up once they are dealt with"
        ));
    }
    Ok(())
}

/// The same conversion, written wherever the caller wants the bytes.
///
/// This is where the work is, and it needs no git — which is what makes the
/// whole of it testable, exactly as `import::from_stream` is. Written this way
/// it is always a whole conversion: nothing is remembered, so nothing is left
/// out.
pub fn to_stream<W: Write>(folder: &Path, out: W) -> Result<Report, Error> {
    convert(folder, out, &Known::nothing()).map(|written| written.report)
}

/// What the repository already holds, and where it stands.
struct Known {
    /// The commits this tool wrote before, that git still holds.
    commits: Commits,
    /// Where the last write left each ref it moved.
    refs: Refs,
    /// Every ref git has now.
    current: BTreeMap<String, Pointed>,
    /// The object ID that names nothing, at this repository's width — what a
    /// ref is reset to when it is being deleted.
    null: String,
}

impl Known {
    fn nothing() -> Self {
        Known {
            commits: Commits::default(),
            refs: Refs::none(),
            current: BTreeMap::new(),
            null: "0".repeat(40),
        }
    }
}

/// What a conversion put in the stream, for the caller that runs git on it.
struct Written {
    report: Report,
    /// Each revision sent, by the mark it was sent under.
    sent: BTreeMap<RevisionId, Mark>,
    /// Every ref this tool now answers for — moved this time, or already
    /// where the store says — and what it points at.
    ours: BTreeMap<String, DataRef>,
    /// Refs moved this time.
    moved: BTreeSet<String>,
    /// Refs deleted this time.
    deleted: BTreeSet<String>,
}

fn convert<W: Write>(folder: &Path, out: W, known: &Known) -> Result<Written, Error> {
    let store = Store::open(folder.join(STORE_DIR))?;
    let mut conversion = Conversion {
        writer: Writer::new(out),
        placed: BTreeMap::new(),
        sent: BTreeMap::new(),
        blobs: BTreeMap::new(),
        next: 0,
        report: Report::default(),
        rewritten: 0,
        contested: BTreeSet::new(),
        private: 0,
        linked: 0,
        invalidated: 0,
        unreadable: BTreeSet::new(),
        branches: Vec::new(),
        ours: BTreeMap::new(),
        moved: BTreeSet::new(),
        deleted: BTreeSet::new(),
        held_back: Vec::new(),
    };
    conversion.run(&store, known)?;
    let branch = checked_out(&conversion.branches);
    let mut report = conversion.report;
    report.branch = branch;
    Ok(Written {
        report,
        sent: conversion.sent,
        ours: conversion.ours,
        moved: conversion.moved,
        deleted: conversion.deleted,
    })
}

/// One conversion in progress.
struct Conversion<W> {
    writer: Writer<W>,
    /// Where each visible revision is in git: the mark it was sent under, or
    /// the object ID the repository already has it as.
    placed: BTreeMap<RevisionId, DataRef>,
    /// The revisions sent this time.
    sent: BTreeMap<RevisionId, Mark>,
    /// A blob per distinct content, so a file unchanged across a thousand
    /// revisions is written once.
    blobs: BTreeMap<RevisionId, Mark>,
    next: u64,
    report: Report,
    rewritten: usize,
    contested: BTreeSet<String>,
    private: usize,
    linked: usize,
    invalidated: usize,
    unreadable: BTreeSet<String>,
    /// Every branch a bookmark named, in the order they were written, so that
    /// one of them can be checked out. Nothing in the store says which — see
    /// [`to_repository`].
    branches: Vec<String>,
    ours: BTreeMap<String, DataRef>,
    moved: BTreeSet<String>,
    deleted: BTreeSet<String>,
    /// Refs the store names that were not moved, and why.
    held_back: Vec<String>,
}

impl<W: Write> Conversion<W> {
    fn run(&mut self, store: &Store, known: &Known) -> Result<(), Error> {
        let history = store.history();
        let superseded = history.superseded();

        // Decision 0004: git has no superseded commit, so what a conversion
        // writes is the current revision of each change.
        let visible: BTreeSet<RevisionId> = history
            .iter()
            .map(|revision| revision.id)
            .filter(|id| !superseded.contains(id))
            .collect();
        self.report.revisions = history.len();

        // Decision 0007: what the repository already holds is named, not sent.
        for id in &visible {
            if let Some(oid) = known.commits.oid_of(id) {
                self.placed.insert(*id, DataRef::Oid(oid.to_owned()));
                self.report.reused += 1;
            }
        }

        for id in order(&history, &visible) {
            if self.placed.contains_key(&id) {
                continue;
            }
            let document = store.get(&id)?.ok_or(Error::Missing { revision: id })?;
            let parents = self.parents(&history, &visible, &superseded, document);
            let tree = self.tree(store, &id)?;
            let changes = self.files(store, &id, &tree)?;
            let author = person(&document.author, &document.when, &id)?;
            let committer = self.committer(document, &author);
            let signature = self.signature(document);

            let mark = self.mark();
            self.write(&Command::Commit(Commit {
                reference: STAGING.as_bytes().to_vec(),
                mark: Some(mark),
                original_oid: None,
                author: Some(author),
                committer,
                encoding: None,
                signature,
                message: document.message.as_bytes().to_vec(),
                from: parents.first().cloned(),
                merges: parents.iter().skip(1).cloned().collect(),
                changes,
            }))?;
            self.placed.insert(id, DataRef::Mark(mark));
            self.sent.insert(id, mark);
            self.report.commits += 1;
        }

        self.refs(store, &history, &visible, known)?;
        self.write(&Command::Done)?;
        self.writer.flush().map_err(Error::Write)?;
        self.finish();
        Ok(())
    }

    /// The files at a revision, as a whole tree.
    ///
    /// `deleteall` and then every file, rather than a difference against the
    /// parent. A git tree is the same tree either way — an object ID is a
    /// function of what the tree *is*, not of how the stream got there — and
    /// stating it whole is the version with no way to be subtly wrong across a
    /// merge, which is the case a difference would have to get right.
    fn files(
        &mut self,
        store: &Store,
        revision: &RevisionId,
        tree: &Tree,
    ) -> Result<Vec<Change>, Error> {
        let mut changes = vec![Change::DeleteAll];
        // By path rather than by identifier, so the stream reads in the order a
        // person would list the folder.
        let mut files: Vec<_> = tree.entries().collect();
        files.sort_by(|(_, a), (_, b)| a.path.cmp(&b.path));

        for (file, entry) in files {
            let (mode, bytes) = match entry.kind {
                // Decision 0040: a link's content is where it points, which is
                // also exactly what git puts in a `120000` blob.
                Kind::Link => {
                    let target = entry.target.as_ref().ok_or_else(|| Error::Contested {
                        revision: *revision,
                        path: entry.path.clone(),
                    })?;
                    let spelled = match target.reference() {
                        // A link historica recorded as an identity, so that it
                        // survives the rename that makes every other tool's
                        // symlink dangle. Git has only the path, so the path is
                        // what it gets — resolved at this revision, which is
                        // the whole point of having kept the identity.
                        Some(named) => {
                            self.linked += 1;
                            tree.path(&named)
                                .ok_or_else(|| Error::Contested {
                                    revision: *revision,
                                    path: entry.path.clone(),
                                })?
                                .to_owned()
                        }
                        None => target.spelling().into_owned(),
                    };
                    (Mode::Symlink, spelled.into_bytes())
                }
                Kind::Lines => {
                    let text = match store.content_at(revision, file)? {
                        Content::Lines(state) => state.text(),
                        Content::Whole(_) => unreachable!("a file of lines holds lines"),
                    };
                    (self.mode(entry), text.into_bytes())
                }
                Kind::Whole => {
                    let payload = entry.payload.ok_or_else(|| Error::Contested {
                        revision: *revision,
                        path: entry.path.clone(),
                    })?;
                    let bytes = store.payload(&payload)?.ok_or_else(|| Error::Absent {
                        revision: *revision,
                        path: entry.path.clone(),
                    })?;
                    (self.mode(entry), bytes)
                }
            };

            let digest = historica::format::digest(&bytes);
            let mark = match self.blobs.get(&digest) {
                Some(mark) => *mark,
                None => {
                    let mark = self.mark();
                    self.write(&Command::Blob(Blob {
                        mark: Some(mark),
                        original_oid: None,
                        data: bytes,
                    }))?;
                    self.blobs.insert(digest, mark);
                    mark
                }
            };
            changes.push(Change::Modify {
                mode,
                content: Carried::Named(DataRef::Mark(mark)),
                path: entry.path.clone().into_bytes(),
            });
        }
        Ok(changes)
    }

    /// Who committed it.
    ///
    /// Decision 0004 restates the author, because a committer read from this
    /// machine's clock and configuration would make the object ID a fact about
    /// who ran the conversion. Where the commit named a different one, import
    /// kept it under `git.committer` — historica's decision 0070 — and this is
    /// where it comes back.
    fn committer(
        &mut self,
        document: &historica::format::RevisionDocument,
        author: &Person,
    ) -> Person {
        let Some(value) = document.extensions.get(carried::COMMITTER) else {
            return author.clone();
        };
        match carried::read_person(value) {
            Some(committer) => committer,
            None => {
                self.unreadable.insert(carried::COMMITTER.to_owned());
                author.clone()
            }
        }
    }

    /// The signature, where the revision still has a right to one.
    ///
    /// A signature is a claim about exact bytes, so a revision that supersedes
    /// another has no business carrying its predecessor's: historica's 0023
    /// carries a header across an amendment because a writer that cannot read
    /// one must not drop it, and 0070 says in as many words that a rewritten
    /// commit's signature is not stale but wrong. Writing it would produce a
    /// commit that says it was signed and was not.
    fn signature(&mut self, document: &historica::format::RevisionDocument) -> Option<Signature> {
        let value = document.extensions.get(carried::SIGNATURE)?;
        if !document.supersedes.is_empty() {
            self.invalidated += 1;
            return None;
        }
        let (Some(data), Some(kind)) = (
            carried::read(value),
            document
                .extensions
                .get(carried::SIGNATURE_KIND)
                .and_then(|kind| carried::read(kind)),
        ) else {
            self.unreadable.insert(carried::SIGNATURE.to_owned());
            return None;
        };
        Some(Signature { kind, data })
    }

    fn mode(&self, entry: &historica::tree::Entry) -> Mode {
        match entry.mode.is_executable() {
            true => Mode::Executable,
            false => Mode::File,
        }
    }

    /// The tree at a revision, with what was decided by rule rather than by
    /// agreement collected on the way past.
    fn tree(&mut self, store: &Store, revision: &RevisionId) -> Result<Tree, Error> {
        let merged = store.merged_tree(revision)?;
        for contest in &merged.contested {
            // Decision 0008 resolves all but one of these by rule and reports
            // it. The one it refuses — two branches each stating a file's whole
            // bytes — is refused here too, in `files`, because git has no way
            // to say a file has no content.
            let said = match contest {
                TreeContest::Dropped { .. } => "a `drop` that lost to concurrent work",
                TreeContest::Moved { .. } => "a file two branches moved to different paths",
                TreeContest::Mode { .. } => "a file two branches gave different modes",
                TreeContest::Content { .. } => continue,
                _ => "a tree fact two branches disagreed about",
            };
            self.contested.insert(said.to_owned());
        }
        Ok(merged.tree)
    }

    /// Where in git this revision's commit stands: on marks sent this time, or
    /// on commits the repository already has.
    ///
    /// A parent that was superseded is not a commit git has, so this walks up
    /// to the nearest ancestor that is one. The shape of the history is kept;
    /// the rewriting that produced it is what does not cross.
    fn parents(
        &mut self,
        history: &historica::core::History,
        visible: &BTreeSet<RevisionId>,
        superseded: &BTreeSet<RevisionId>,
        document: &historica::format::RevisionDocument,
    ) -> Vec<DataRef> {
        let mut parents = Vec::new();
        for parent in &document.parents {
            let mut standing = *parent;
            let mut guard = 0;
            while !visible.contains(&standing) && superseded.contains(&standing) {
                self.rewritten += 1;
                let Some(next) = history
                    .get(&standing)
                    .and_then(|revision| revision.parents.iter().next().copied())
                else {
                    break;
                };
                standing = next;
                guard += 1;
                if guard > history.len() {
                    break;
                }
            }
            if let Some(placed) = self.placed.get(&standing)
                && !parents.contains(placed)
            {
                parents.push(placed.clone());
            }
        }
        parents
    }

    /// Move the refs, delete the ones this tool made that nothing names any
    /// more, then take the staging ref away.
    fn refs(
        &mut self,
        store: &Store,
        history: &historica::core::History,
        visible: &BTreeSet<RevisionId>,
        known: &Known,
    ) -> Result<(), Error> {
        let mut named: BTreeSet<RevisionId> = BTreeSet::new();
        let mut wanted: Vec<(String, DataRef, RevisionId, bool)> = Vec::new();

        for (name, bookmark) in store.names() {
            // Historica's decision 0062: a private bookmark's name stays
            // behind. An exporter that shipped it would be publishing the one
            // thing its owner said not to.
            if !bookmark.travels() {
                self.private += 1;
                continue;
            }
            let Some((target, directory)) = resolve(history, &bookmark.target) else {
                continue;
            };
            let Some(placed) = self.placed.get(&target).cloned() else {
                continue;
            };
            let Some(reference) = git_ref(directory, name) else {
                self.report.note(format!(
                    "the bookmark `{name}` did not cross; git has no ref by that name"
                ));
                continue;
            };
            wanted.push((reference, placed, target, directory == HEADS));
        }

        for (reference, placed, target, branch) in wanted {
            if self.place(&reference, placed, known)? {
                named.insert(target);
                if branch {
                    self.branches.push(reference[HEADS.len()..].to_owned());
                }
            }
        }

        // A head nobody bookmarked — or whose ref could not be moved — is
        // still work somebody did, and a commit no ref reaches is a commit git
        // will collect. So it goes somewhere out of the way rather than
        // nowhere.
        for head in history.heads() {
            if !visible.contains(&head) || named.contains(&head) {
                continue;
            }
            let Some(placed) = self.placed.get(&head).cloned() else {
                continue;
            };
            let reference = format!("{UNNAMED}/{}", head.abbreviate(12));
            self.place(&reference, placed, known)?;
        }

        // What a write of this tool's made and the store no longer names.
        // Deleted where git still has it as it was left; otherwise somebody
        // else's now. A ref the record merely found in agreement — one git
        // had before this tool ever wrote here — is not this tool's to delete,
        // however the store feels about it.
        for (reference, left) in known.refs.iter() {
            if !left.made || self.ours.contains_key(reference) || self.held_back_names(reference) {
                continue;
            }
            match known.current.get(reference) {
                Some(pointed) if pointed.commit == left.oid => {
                    self.write(&Command::Reset(Reset {
                        reference: reference.as_bytes().to_vec(),
                        from: Some(DataRef::Oid(known.null.clone())),
                    }))?;
                    self.report.deleted.push(reference.to_owned());
                    self.deleted.insert(reference.to_owned());
                }
                Some(_) => self.held_back.push(format!(
                    "`{reference}` was not deleted: the store no longer names it, but \
                     git moved it since the last write, so it is somebody's now"
                )),
                None => {}
            }
        }

        // Take the staging ref away, which fast-import spells as a reset to the
        // null object ID. Only where something was parked there: a conversion
        // that sent nothing never made it.
        if !self.sent.is_empty() {
            self.write(&Command::Reset(Reset {
                reference: STAGING.as_bytes().to_vec(),
                from: Some(DataRef::Oid(known.null.clone())),
            }))?;
        }
        Ok(())
    }

    /// Move one ref, or note that it is already right, or say why neither.
    /// Answers whether the ref now stands where the store says.
    fn place(&mut self, reference: &str, target: DataRef, known: &Known) -> Result<bool, Error> {
        match self.decide(reference, &target, known) {
            Decided::Move => {
                self.write(&Command::Reset(Reset {
                    reference: reference.as_bytes().to_vec(),
                    from: Some(target.clone()),
                }))?;
                self.report.references.push(reference.to_owned());
                self.moved.insert(reference.to_owned());
                self.ours.insert(reference.to_owned(), target);
                Ok(true)
            }
            Decided::Already => {
                self.ours.insert(reference.to_owned(), target);
                Ok(true)
            }
            Decided::HeldBack(because) => {
                self.held_back
                    .push(format!("`{reference}` was not moved: {because}"));
                Ok(false)
            }
        }
    }

    /// Whether a ref the store names has already been reported held back.
    fn held_back_names(&self, reference: &str) -> bool {
        let quoted = format!("`{reference}`");
        self.held_back.iter().any(|line| line.starts_with(&quoted))
    }

    /// Decision 0007's rule for one ref: the side that moved it since the last
    /// write is the side that is right, and git having moved it is a reason to
    /// `import` before writing rather than a reason to overwrite.
    fn decide(&self, reference: &str, target: &DataRef, known: &Known) -> Decided {
        let now = known.current.get(reference);
        let left = known.refs.left(reference);
        match now {
            None => match left {
                // Git deleted a ref this tool made. The store still names it,
                // and recreating it would undo a deletion somebody meant.
                Some(_) => Decided::HeldBack(
                    "git deleted it since the last write, and the store still names \
                     it; `import` carries the deletion across, or point the bookmark \
                     somewhere and write again"
                        .to_owned(),
                ),
                None => Decided::Move,
            },
            Some(pointed) => {
                if pointed.annotated {
                    return Decided::HeldBack(
                        "git has it as an annotated tag, which is an object of its own \
                         and not this tool's to replace"
                            .to_owned(),
                    );
                }
                if let DataRef::Oid(oid) = target
                    && *oid == pointed.commit
                {
                    return Decided::Already;
                }
                match left {
                    Some(left) if left == pointed.commit => Decided::Move,
                    Some(_) => Decided::HeldBack(
                        "git moved it since the last write; `import` first, so that \
                         the store sees where it went"
                            .to_owned(),
                    ),
                    None => Decided::HeldBack(
                        "git already has it, pointing elsewhere, and this tool never \
                         wrote it; `import` first, so that the store sees it"
                            .to_owned(),
                    ),
                }
            }
        }
    }

    fn mark(&mut self) -> Mark {
        self.next += 1;
        Mark(self.next)
    }

    fn write(&mut self, command: &Command) -> Result<(), Error> {
        self.writer.write(command).map_err(Error::Write)
    }

    fn finish(&mut self) {
        let uncrossed = self.report.revisions - self.report.commits - self.report.reused;
        if uncrossed > 0 {
            self.report.note(if uncrossed == 1 {
                "one revision was superseded by another and did not cross; git has \
                 no commit for a rewriting, so what crossed is the version that \
                 stands now"
                    .to_owned()
            } else {
                format!(
                    "{uncrossed} revisions were superseded by others and did not \
                     cross; git has no commit for a rewriting, so what crossed is \
                     the version that stands now"
                )
            });
        }
        if self.rewritten > 0 {
            self.report.note(
                "a commit stands on the nearest ancestor git has, because the \
                 revision it was recorded against was rewritten away"
                    .to_owned(),
            );
        }
        for said in std::mem::take(&mut self.contested) {
            self.report.note(format!(
                "{said} was resolved by historica's rule and git records only the \
                 outcome; nothing says a choice was made"
            ));
        }
        if self.linked > 0 {
            self.report.note(
                "a link historica holds as an identity became the path that file is \
                 at now, which is the path git would have had only if nobody had \
                 renamed the file since — decision 0040 keeps such a link pointing \
                 at its file, and git's would be left dangling"
                    .to_owned(),
            );
        }
        if self.invalidated > 0 {
            let invalidated = self.invalidated;
            self.report.note(if invalidated == 1 {
                "one revision carries a signature over a commit it is no longer \
                 the same work as, because it was amended after it was imported; \
                 the signature did not cross, since a commit that says it was \
                 signed and was not is worse than one that says nothing"
                    .to_owned()
            } else {
                format!(
                    "{invalidated} revisions carry signatures over commits they are \
                     no longer the same work as, because they were amended after \
                     they were imported; the signatures did not cross, since a \
                     commit that says it was signed and was not is worse than one \
                     that says nothing"
                )
            });
        }
        for key in std::mem::take(&mut self.unreadable) {
            self.report.note(format!(
                "a `{key}` header did not cross; something wrote one under a key \
                 this tool owns and it does not say what this tool writes"
            ));
        }
        if self.private > 0 {
            let private = self.private;
            self.report.note(if private == 1 {
                "one bookmark is private and its name stayed behind".to_owned()
            } else {
                format!("{private} bookmarks are private and their names stayed behind")
            });
        }
        for line in std::mem::take(&mut self.held_back) {
            self.report.note(line);
        }
        self.report.note(
            "change IDs did not cross; git has nowhere to put one, and decision 0003 \
             derives one from a commit rather than the other way about"
                .to_owned(),
        );
    }
}

/// What to do with one ref the store names.
enum Decided {
    Move,
    Already,
    HeldBack(String),
}

/// The marks file `git fast-import --export-marks` writes: `:<mark> <oid>`.
fn read_marks(at: &Path) -> Result<BTreeMap<Mark, String>, Error> {
    let text = match std::fs::read_to_string(at) {
        Ok(text) => text,
        // Nothing was sent, so nothing was marked, and git may write no file.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(Error::io(at, error)),
    };
    let mut marks = BTreeMap::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed = line
            .split_once(' ')
            .and_then(|(mark, oid)| Some((mark.strip_prefix(':')?.parse::<u64>().ok()?, oid)));
        let Some((mark, oid)) = parsed else {
            return Err(Error::Marks {
                line: line.to_owned(),
            });
        };
        marks.insert(Mark(mark), oid.to_owned());
    }
    Ok(marks)
}

/// Which branch a fresh repository has checked out.
///
/// Nothing in the store says, so the conversion picks and says which. `main`
/// and then `master` before anything else, because they are what a person means
/// by "the branch" and taking the first in name order would not be: once a
/// bookmark's name may hold a `/`, `claude/something` sorts above `main`, and a
/// conversion that checked out somebody's scratch branch would be technically
/// arbitrary and practically wrong.
fn checked_out(branches: &[String]) -> Option<String> {
    for usual in ["main", "master"] {
        if let Some(found) = branches.iter().find(|branch| *branch == usual) {
            return Some(found.clone());
        }
    }
    branches.first().cloned()
}

/// The revision a bookmark points at, and the ref it becomes.
///
/// Decision 0006: historica's two kinds of target are the distinction git
/// spells with two directories, which is what lets a branch and a tag both
/// cross without this inventing a grammar for either.
fn resolve(
    history: &historica::core::History,
    name: &historica::store::Name,
) -> Option<(RevisionId, &'static str)> {
    match name {
        // A bookmark on a change follows the work through every rewrite, which
        // is what a branch does.
        historica::store::Name::Change(change) => match history.change_state(change) {
            ChangeState::Resolved(revision) => Some((revision.id, HEADS)),
            _ => None,
        },
        // A bookmark on a revision is pinned and cannot move, which is what a
        // tag is.
        historica::store::Name::Revision(revision) => Some((*revision, TAGS)),
        // A bookmark on a file is not a place in the history, so there is no
        // ref it could become.
        historica::store::Name::File(_) => None,
    }
}

/// Parents before children, and otherwise by digest so the order is the store's
/// rather than the filesystem's.
fn order(history: &historica::core::History, visible: &BTreeSet<RevisionId>) -> Vec<RevisionId> {
    let mut placed = BTreeSet::new();
    let mut out = Vec::new();
    // Repeated sweeps rather than a worklist: the set is already sorted by
    // digest, so a sweep places everything whose parents are placed and keeps
    // the order stable between runs, which decision 0004 needs.
    loop {
        let mut moved = false;
        for id in visible {
            if placed.contains(id) {
                continue;
            }
            let ready = history
                .get(id)
                .map(|revision| {
                    revision
                        .parents
                        .iter()
                        .all(|parent| placed.contains(parent) || !visible.contains(parent))
                })
                .unwrap_or(true);
            if ready {
                placed.insert(*id);
                out.push(*id);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    out
}

/// `Name <email>` and a timestamp, as git's two numbers.
fn person(
    author: &str,
    when: &historica::format::Timestamp,
    revision: &RevisionId,
) -> Result<Person, Error> {
    let (name, email) = match (author.rfind('<'), author.ends_with('>')) {
        (Some(at), true) => (
            author[..at].trim_end().as_bytes().to_vec(),
            author.as_bytes()[at + 1..author.len() - 1].to_vec(),
        ),
        // Decision 0010 makes an author a string a person chose, and git wants
        // two fields. A name with no address keeps the name and states an empty
        // address, which is a thing git allows and which loses nothing.
        _ => (author.as_bytes().to_vec(), Vec::new()),
    };

    // The spelling is fixed at 25 characters — `YYYY-MM-DDThh:mm:ss±hh:mm` —
    // so the offset is the last six of them and needs no parser.
    let spelled = when.as_str();
    let moment: jiff::Timestamp = spelled.parse().map_err(|_| Error::Time {
        revision: *revision,
        when: spelled.to_owned(),
    })?;
    let offset = &spelled[spelled.len() - 6..];
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let hours: i32 = offset[1..3].parse().map_err(|_| Error::Time {
        revision: *revision,
        when: spelled.to_owned(),
    })?;
    let minutes: i32 = offset[4..6].parse().map_err(|_| Error::Time {
        revision: *revision,
        when: spelled.to_owned(),
    })?;

    Ok(Person {
        name,
        email,
        seconds: moment.as_second(),
        offset_minutes: sign * (hours * 60 + minutes),
    })
}

/// A bookmark's name as a git ref, or nothing where git would refuse it.
fn git_ref(directory: &str, name: &str) -> Option<String> {
    let refused = name.is_empty()
        || name.starts_with('-')
        || name.starts_with('.')
        || name.ends_with('.')
        || name.ends_with(".lock")
        || name.ends_with('/')
        || name.contains("..")
        || name.contains("//")
        || name.contains("@{")
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\'));
    match refused {
        true => None,
        false => Some(format!("{directory}{name}")),
    }
}

/// The target must be empty or absent, for the reason import's must be.
fn free(at: &Path) -> Result<(), Error> {
    match std::fs::read_dir(at) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(at, error)),
        Ok(mut entries) => match entries.next() {
            None => Ok(()),
            Some(_) => Err(Error::NotFree {
                at: at.to_path_buf(),
                because: "there is already something here".to_owned(),
            }),
        },
    }
}
