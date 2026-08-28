//! A git repository, converted into a Historica store.
//!
//! The shape decision 0002 leaves: `git fast-export` writes a stream, the
//! reader parses it, the replay works out the tree at each commit, and this
//! puts that tree in a folder and asks [`historica::record`] to record it.
//! historica does its own diffing — it reads a folder, not a patch — so the
//! conversion's job is to present each commit as a folder and to state the
//! facts a folder cannot carry: who wrote it, when, what was renamed, and what
//! it stands on.
//!
//! Decision 0007 lets a conversion run onto a store it made before. The
//! commits the store already holds are left out of the export, a commit
//! standing on one names it by object ID, and the folder — a person's working
//! copy, by then — is not touched: what a second conversion writes goes into
//! `history/` and nowhere else, and `historica update` is what brings the
//! folder forward.
//!
//! Nothing of a person's is ever deleted here. A fresh target must be empty or
//! absent, and the only paths this removes are ones it wrote itself on a
//! previous commit.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::rc::Rc;

use historica::core::{ChangeState, History, RevisionId};
use historica::format::Timestamp;
use historica::record::{self, Recording, Restriction};
use historica::store::{Bookmark, Name, STORE_DIR, Store, StoreError};
use historica::working::{Skipped, Working};

use crate::carried;
use crate::identity::Identity;
use crate::plumbing::{Pointed, Repository};
use crate::remembered::{Commits, Left, Refs};
use crate::stream::{Change, Command, Commit, DataRef, Mark, Person, Reader};
use crate::tree::{Tree, Trees};

mod error;
mod folder;

pub use error::Error;

/// What a conversion did, and what it could not carry.
///
/// Decision 0001 requires the second half: each side holds facts the other has
/// no place for, and dropping them quietly is the failure that matters, because
/// the person holding the result believes it is the thing it came from.
#[derive(Clone, Default, Debug)]
pub struct Report {
    /// Commits read from the stream.
    pub commits: usize,
    /// Revisions written to the store.
    pub revisions: usize,
    /// Commits that changed nothing and are not a merge, which historica has
    /// no revision for. Their descendants stand on their parent instead.
    pub empty: usize,
    /// Commits the store already held, which were recognised and not
    /// converted again.
    pub held: usize,
    /// Bookmarks named at a converted revision.
    pub bookmarks: Vec<String>,
    /// Bookmarks that existed and were moved to where git's ref now is.
    pub moved: Vec<String>,
    /// Whether the store existed before this conversion, in which case the
    /// folder beside it was left exactly as it was.
    pub onto: bool,
    /// One line per kind of fact that did not cross, for a person to read.
    pub uncarried: Vec<String>,
}

impl Report {
    fn note(&mut self, line: String) {
        self.uncarried.push(line);
    }
}

/// Convert the repository at `repository` into the store under `folder`.
///
/// A folder holding no store gets a fresh one and ends as the last commit
/// left it. A folder holding a store this tool made before gets what the
/// repository has gained since, and nothing else about it is touched.
///
/// Runs `git fast-export` and reads what it writes. The flags are decision
/// 0002's and 0003's: `-M` so a rename arrives as a rename,
/// `--show-original-ids` because a change ID is derived from the object ID, and
/// the two signature modes because git otherwise refuses outright at the first
/// signed tag. Decision 0007 adds the commits already held as exclusions, and
/// `--reference-excluded-parents` so that what stands on them says so.
pub fn from_repository(repository: &Path, folder: &Path) -> Result<Report, Error> {
    let repository = Repository::at(repository).ok_or_else(|| Error::NotARepository {
        at: repository.to_path_buf(),
    })?;
    let onto = folder.join(STORE_DIR).exists();

    let mut store = if onto {
        Store::open(folder.join(STORE_DIR))?
    } else {
        folder::must_be_free(folder)?;
        fs::create_dir_all(folder).map_err(|error| Error::io(folder, error))?;
        Store::init(folder.join(STORE_DIR))?
    };

    let git_dir = repository.git_dir()?;
    let known = Known {
        commits: Commits::read(&git_dir)?,
        refs: Refs::read(&git_dir)?,
        history: store.history(),
    };
    let current = repository.refs()?;

    // Every ref whose commit the store already holds is excluded from the
    // export, and with it everything beneath. A held commit that is reachable
    // only through a ref that has since moved on still arrives, and is
    // recognised when it does.
    let mut arguments = vec![
        "fast-export".to_owned(),
        "--all".to_owned(),
        "-M".to_owned(),
        "--show-original-ids".to_owned(),
        "--reencode=yes".to_owned(),
        // Decision 0004's measurement was that a signed commit does not
        // round-trip, and historica's 0070 opened the door that closes it:
        // the signature comes across verbatim and is recorded as a header
        // this tool owns. `strip` would throw away the one fact that makes
        // the commit what it is.
        "--signed-commits=verbatim".to_owned(),
        "--signed-tags=strip".to_owned(),
        "--reference-excluded-parents".to_owned(),
    ];
    for pointed in current.values() {
        if known.revision_of(&pointed.commit).is_some() {
            arguments.push(format!("^{}", pointed.commit));
        }
    }

    let mut child = Process::new("git")
        .arg("-C")
        .arg(repository.path())
        .args(&arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Error::Spawn)?;

    // Drained on its own thread. git writes a warning per signed commit, and a
    // pipe nobody reads fills and stops the export halfway through.
    let complaints = child.stderr.take().expect("stderr was piped");
    let listening = std::thread::spawn(move || {
        BufReader::new(complaints)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
    });

    // A second conversion materialises each commit somewhere of its own. The
    // folder beside the store is a person's working copy by now, and historica's
    // 0029 keeps working folders outside a receive; this is the same rule.
    let scratch = onto.then(Scratch::make).transpose()?;
    let materialise_in = match &scratch {
        Some(scratch) => scratch.path().to_path_buf(),
        None => folder.to_path_buf(),
    };

    let stream = child.stdout.take().expect("stdout was piped");
    let outcome = convert(
        BufReader::new(stream),
        &mut store,
        &materialise_in,
        known,
        Some(&repository),
        Some(current),
    );
    drop(scratch);

    let said = listening.join().unwrap_or_default();
    let status = child.wait().map_err(Error::Spawn)?;
    let converted = outcome?;
    if !status.success() {
        return Err(Error::GitFailed {
            status: status.code(),
            said,
        });
    }

    // The repository's record, kept by both conversions. A commit recorded
    // here is a revision the store now holds, and a write that learned that
    // names it rather than sending it again; a ref the store now agrees with
    // is one a write may move, rather than one git moved behind its back.
    let mut commits = converted.known.commits;
    for (oid, revision) in &converted.recorded {
        commits.insert(*revision, oid.clone());
    }
    commits.write(&git_dir)?;
    // Carried forward and amended, never rewritten from scratch: a ref that
    // was held back keeps its line, and a ref a write made stays marked as
    // made even once an import finds the two sides agreeing about it again.
    let mut left = converted.known.refs.to_map();
    for (reference, oid) in converted.agreed {
        let made = converted.known.refs.made(&reference);
        left.insert(reference, Left { oid, made });
    }
    Refs::write(&git_dir, &left)?;

    let mut report = converted.report;
    report.onto = onto;
    Ok(report)
}

/// The same conversion, from a stream somebody else produced.
///
/// This is where the work is, and it needs no git: a stream in a file converts
/// exactly as a stream from a pipe does, which is what makes the whole of it
/// testable against the corpus. It is always a fresh conversion: a stream
/// names nothing this tool could ask git about, so a commit it only names is
/// refused rather than looked up.
pub fn from_stream<R: BufRead>(input: R, folder: &Path) -> Result<Report, Error> {
    folder::must_be_free(folder)?;
    fs::create_dir_all(folder).map_err(|error| Error::io(folder, error))?;
    let mut store = Store::init(folder.join(STORE_DIR))?;
    let known = Known {
        commits: Commits::default(),
        refs: Refs::none(),
        history: store.history(),
    };
    convert(input, &mut store, folder, known, None, None).map(|converted| converted.report)
}

/// What a conversion leaves for the repository's record, beside its report.
struct Converted {
    report: Report,
    known: Known,
    /// Every commit recorded this time, and the revision it became. Not the
    /// commits that changed nothing: those became their parent, and a record
    /// naming two commits for one revision would answer the reverse question
    /// two ways.
    recorded: BTreeMap<String, RevisionId>,
    /// Every ref whose bookmark now says what git says, and the commit both
    /// point at.
    agreed: BTreeMap<String, String>,
}

/// What the store and the repository already know about each other.
struct Known {
    /// The commits this tool wrote, from the repository's own record.
    commits: Commits,
    /// Where the last write left each ref.
    refs: Refs,
    /// The store's history as it stood before this conversion.
    history: History,
}

impl Known {
    /// The revision a commit already is, if the store holds it.
    ///
    /// Two ways to be held. A commit this tool *wrote* is in the repository's
    /// own record; a commit this tool *imported* has a change ID that is its
    /// object ID's first twelve bytes (decision 0003), and the revision that
    /// came from the commit is the one of that change that supersedes nothing.
    fn revision_of(&self, oid: &str) -> Option<RevisionId> {
        if let Some(revision) = self.commits.revision_of(oid) {
            return Some(revision);
        }
        let change = Identity::from_oid(oid).ok()?.change_id();
        let mut originals = self
            .history
            .revisions_of(&change)
            .filter(|revision| revision.supersedes.is_empty());
        let first = originals.next()?;
        // A change with two revisions that supersede nothing is one two
        // conversions disagreed about, which decision 0003 says cannot happen;
        // it is treated as not held rather than guessed between.
        originals.next().is_none().then_some(first.id)
    }
}

/// A folder that exists for one conversion and is removed after it.
struct Scratch(PathBuf);

impl Scratch {
    fn make() -> Result<Self, Error> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default();
        let at = std::env::temp_dir().join(format!("historica-git-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&at).map_err(|error| Error::io(&at, error))?;
        Ok(Scratch(at))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn convert<R: BufRead>(
    input: R,
    store: &mut Store,
    folder: &Path,
    known: Known,
    repository: Option<&Repository>,
    current: Option<BTreeMap<String, Pointed>>,
) -> Result<Converted, Error> {
    let mut conversion = Conversion {
        repository,
        folder: folder.to_path_buf(),
        trees: Trees::new(),
        written: Rc::new(Tree::default()),
        revisions: BTreeMap::new(),
        by_oid: BTreeMap::new(),
        recorded: BTreeMap::new(),
        known,
        report: Report::default(),
        committers: 0,
        signatures: 0,
        encodings: 0,
        tags: 0,
        refs: BTreeMap::new(),
        ref_oids: BTreeMap::new(),
        agreed: BTreeMap::new(),
        unnameable: Vec::new(),
        elsewhere: BTreeSet::new(),
        dangling: 0,
        conflicts: Vec::new(),
    };

    for command in Reader::new(input) {
        let command = command?;
        match &command {
            Command::Commit(commit) => {
                // A parent the export left out is named rather than carried,
                // and the replay needs its tree before this commit can be
                // applied to it. Git is the only party that has it exactly.
                for named in commit.from.iter().chain(&commit.merges) {
                    if let DataRef::Oid(oid) = named
                        && !conversion.trees.is_seeded(oid)
                    {
                        let Some(repository) = repository else {
                            return Err(Error::ParentNotCarried);
                        };
                        let tree = repository.tree_of(oid)?;
                        conversion.trees.seed(oid.clone(), tree);
                    }
                }
            }
            // A ref left pointing at an excluded commit. The replay has no
            // tree for it and needs none — git emits these after the last
            // commit — and where the ref stands is read from git rather than
            // from here. Only where git *can* be read: a stream on its own that
            // names a commit it did not carry is still refused, by the replay.
            Command::Reset(reset)
                if repository.is_some() && matches!(reset.from, Some(DataRef::Oid(_))) =>
            {
                continue;
            }
            _ => {}
        }
        // Asked before the commit is applied, because applying it moves the
        // ref it continues — see `Trees::parent_of`.
        let parent = match &command {
            Command::Commit(commit) => Some(conversion.trees.parent_of(commit)?),
            _ => None,
        };
        let tree = conversion.trees.apply(&command)?;
        match (&command, tree, parent) {
            (Command::Commit(commit), Some(tree), Some(parent)) => {
                conversion.commit(store, commit, tree, parent)?;
            }
            // An annotated tag is an object with a tagger and a message of its
            // own, and historica has nowhere for either. Its `from` is not
            // followed into a bookmark, because a bookmark would say the tag
            // crossed when what crossed is where it pointed. Counted here only
            // where the stream is the sole account of the refs; where git is
            // asked, git's answer counts them once.
            (Command::Tag(_), _, _) if current.is_none() => conversion.tags += 1,
            (Command::Reset(reset), _, _) => conversion.moved(reset),
            _ => {}
        }
    }

    // Where the refs stand: from git where git can be asked, since an
    // excluded commit's ref never reaches the stream; from the stream
    // otherwise.
    if let Some(current) = current {
        conversion.refs.clear();
        for (reference, pointed) in current {
            if pointed.annotated {
                if reference.starts_with("refs/tags/") {
                    conversion.tags += 1;
                }
                continue;
            }
            match conversion.resolve(&pointed.commit) {
                Some(revision) => {
                    conversion
                        .ref_oids
                        .insert(reference.clone(), pointed.commit);
                    conversion.refs.insert(reference, revision);
                }
                None => conversion.dangling += 1,
            }
        }
    }
    conversion.name(store)?;
    let recorded = std::mem::take(&mut conversion.recorded);
    let agreed = std::mem::take(&mut conversion.agreed);
    let known = std::mem::replace(
        &mut conversion.known,
        Known {
            commits: Commits::default(),
            refs: Refs::none(),
            history: History::default(),
        },
    );
    Ok(Converted {
        report: conversion.finish(),
        known,
        recorded,
        agreed,
    })
}

/// One conversion in progress.
struct Conversion<'a> {
    /// The repository, where there is one to ask questions of.
    repository: Option<&'a Repository>,
    folder: PathBuf,
    trees: Trees,
    /// What is on disk right now, so that materialising the next commit writes
    /// the difference rather than the whole tree.
    written: Rc<Tree>,
    /// Where each commit of the stream landed in the store.
    revisions: BTreeMap<Mark, RevisionId>,
    /// The same, by object ID, for the refs git is asked about afterwards.
    by_oid: BTreeMap<String, RevisionId>,
    /// The commits actually recorded this time, for the repository's record.
    recorded: BTreeMap<String, RevisionId>,
    known: Known,
    report: Report,
    committers: usize,
    signatures: usize,
    encodings: usize,
    tags: usize,
    /// Where each ref of the stream ended up, played forward exactly as the
    /// stream moves them: a `commit` moves the ref it names, and a `reset`
    /// moves the ref it names to wherever it says.
    refs: BTreeMap<String, RevisionId>,
    /// The commit each of git's refs points at, where git was asked.
    ref_oids: BTreeMap<String, String>,
    /// Every ref whose bookmark now says what git says.
    agreed: BTreeMap<String, String>,
    unnameable: Vec<(String, String)>,
    elsewhere: BTreeSet<String>,
    dangling: usize,
    /// Bookmarks that could not be moved, and why.
    conflicts: Vec<String>,
}

impl Conversion<'_> {
    fn commit(
        &mut self,
        store: &mut Store,
        commit: &Commit,
        tree: Rc<Tree>,
        parent: Rc<Tree>,
    ) -> Result<(), Error> {
        self.report.commits += 1;
        let mut identity = Identity::of(commit)?;
        let oid = commit
            .original_oid
            .clone()
            .expect("checked by Identity::of");
        let reference = String::from_utf8_lossy(&commit.reference).into_owned();

        // A commit the store already holds arrives when it is reachable only
        // through a ref that has moved on. It is recognised by what it is
        // rather than converted again, and what stands on it stands on the
        // revision it already is.
        if let Some(revision) = self.known.revision_of(&oid) {
            self.report.held += 1;
            self.landed(commit, &reference, &oid, revision, false);
            return Ok(());
        }

        // The folder, as this commit has it.
        let on_disk = Rc::clone(&self.written);
        folder::materialise(&self.folder, &on_disk, &tree)?;
        self.written = Rc::clone(&tree);

        let working = Working::read(&self.folder, &Skipped::none())?;
        let parents = self.parents(commit)?;
        // A rename is expanded against the tree the commit started from —
        // its parent's, which the replay knows — rather than against whatever
        // was on disk last, which is the previous commit in *stream* order
        // and another branch's whenever the stream switches.
        let moves = self.moves(commit, &parent)?;

        // Git states an author and a committer; historica records one person,
        // and the author is the one it keeps. The committer is not thrown away
        // for it — historica's decision 0070 lets this tool state a header of
        // its own, and the committer is in the commit's bytes, so a revision
        // that could not carry it could not be written back as the commit it
        // came from.
        let author = commit.author.as_ref().unwrap_or(&commit.committer);
        let mut extensions = BTreeMap::new();
        if commit.author.is_some() && commit.author.as_ref() != Some(&commit.committer) {
            self.committers += 1;
            extensions.insert(
                carried::COMMITTER.to_owned(),
                carried::spell_person(&commit.committer),
            );
        }
        if let Some(signature) = &commit.signature {
            self.signatures += 1;
            extensions.insert(
                carried::SIGNATURE.to_owned(),
                carried::spell(&signature.data),
            );
            extensions.insert(
                carried::SIGNATURE_KIND.to_owned(),
                carried::spell(&signature.kind),
            );
        }
        if commit.encoding.is_some() {
            self.encodings += 1;
        }

        let recording = Recording {
            parents,
            author: spell(author),
            when: when(author)?,
            message: String::from_utf8_lossy(&commit.message).into_owned(),
            moves,
            at: Vec::new(),
            accepted: Default::default(),
            only: Restriction::Everything,
            extensions,
            // Nothing stated, so every added file is sniffed. Git has no
            // better answer to hand over: a blob carries a mode and bytes and
            // no notion of text, and git's own tools sniff exactly as
            // historica's do. `.gitattributes` is the one place a repository
            // ever says otherwise, and reading it is a feature this importer
            // does not have rather than a fact it is discarding here.
            kinds: Default::default(),
        };

        // Historica's decision 0011 has `record` carry forward every bookmark
        // that named a parent's change, which is right for a person recording
        // onto a branch and wrong here: where git's refs stand is git's to
        // say, and is said once the stream is read. So what `record` advanced
        // is put back where it was, and the one thing a conversion does to a
        // bookmark is done in `name`.
        let standing: BTreeMap<String, Name> = {
            let followed: BTreeSet<_> = recording
                .parents
                .iter()
                .filter_map(|parent| store.revision(parent).map(|revision| revision.change))
                .collect();
            store
                .names()
                .iter()
                .filter(|(_, bookmark)| match &bookmark.target {
                    Name::Change(change) => followed.contains(change),
                    _ => false,
                })
                .map(|(name, bookmark)| (name.clone(), bookmark.target))
                .collect()
        };

        match record::record(store, &working, &recording, &mut identity) {
            Ok(recorded) => {
                self.report.revisions += 1;
                for name in &recorded.advanced {
                    if let Some(target) = standing.get(name) {
                        store.set_name(name, *target)?;
                    }
                }
                self.landed(commit, &reference, &oid, recorded.revision, true);
            }
            // A commit that changed nothing and joins nothing is not a
            // revision historica can write — `record` says so rather than
            // writing an empty one. Its descendants stand on its parent, so
            // the shape of the history is kept even though the commit is not.
            Err(record::RecordError::NothingToRecord) => {
                self.report.empty += 1;
                if let Some(parent) = recording.parents.first() {
                    self.landed(commit, &reference, &oid, *parent, false);
                }
            }
            Err(other) => return Err(Error::Record(Box::new(other))),
        }
        Ok(())
    }

    /// Note where a commit landed, by every name the rest of the stream or
    /// git might use for it — and, where it was recorded rather than
    /// recognised or folded into its parent, for the repository's record.
    fn landed(
        &mut self,
        commit: &Commit,
        reference: &str,
        oid: &str,
        revision: RevisionId,
        recorded: bool,
    ) {
        self.refs.insert(reference.to_owned(), revision);
        self.by_oid.insert(oid.to_owned(), revision);
        if recorded {
            self.recorded.insert(oid.to_owned(), revision);
        }
        if let Some(mark) = commit.mark {
            self.revisions.insert(mark, revision);
        }
    }

    /// The revision an object ID is, whether it landed in this conversion or
    /// was already held.
    fn resolve(&self, oid: &str) -> Option<RevisionId> {
        self.by_oid
            .get(oid)
            .copied()
            .or_else(|| self.known.revision_of(oid))
    }

    /// A `reset`: a ref moved with no commit of its own.
    ///
    /// Lightweight tags and branch creation both arrive this way, and a reset
    /// with no `from` only deletes, which leaves the ref where the stream has
    /// nothing further to say about it.
    fn moved(&mut self, reset: &crate::stream::Reset) {
        let reference = String::from_utf8_lossy(&reset.reference).into_owned();
        match &reset.from {
            Some(DataRef::Mark(mark)) => match self.revisions.get(mark) {
                Some(revision) => {
                    self.refs.insert(reference, *revision);
                }
                None => self.dangling += 1,
            },
            // An object ID rather than a mark: content this stream did not
            // carry, so there is no revision here to point at.
            Some(DataRef::Oid(_)) => self.dangling += 1,
            None => {
                self.refs.remove(&reference);
            }
        }
    }

    /// Name a bookmark at each ref, or move the one that is there.
    ///
    /// Decision 0006: a branch follows the work, so it becomes a bookmark on
    /// the change; a tag names one version and does not move, so it becomes a
    /// bookmark on the revision. Historica's two kinds of target are the
    /// distinction git spells with two directories, which is what lets both
    /// cross without this inventing a grammar for either.
    ///
    /// Decision 0007, for a bookmark that already exists and points elsewhere:
    /// where the repository remembers where the last write left the ref, the
    /// side that moved is the side that is right, and both having moved is a
    /// conflict said out loud. Where it remembers nothing, a bookmark follows
    /// the ref only if that is a step forward along its own history.
    fn name(&mut self, store: &mut Store) -> Result<(), Error> {
        let history = store.history();
        for (reference, revision) in std::mem::take(&mut self.refs) {
            let (name, target, pinned) = if let Some(branch) = reference.strip_prefix("refs/heads/")
            {
                let Some(change) = store.revision(&revision).map(|revision| revision.change) else {
                    self.dangling += 1;
                    continue;
                };
                (branch.to_owned(), Name::Change(change), false)
            } else if let Some(tag) = reference.strip_prefix("refs/tags/") {
                (tag.to_owned(), Name::Revision(revision), true)
            } else {
                // `refs/remotes/`, `refs/notes/`, and whatever else a
                // repository keeps. Each is a fact about somewhere else, and a
                // bookmark that claimed otherwise would be this tool deciding
                // what somebody's remote-tracking ref meant.
                self.elsewhere.insert(elsewhere(&reference));
                continue;
            };

            let outcome = match store.bookmark(&name) {
                None => match store.set_bookmark(&name, Bookmark::shared(target)) {
                    Ok(()) => Ok(Named::New),
                    Err(error) => Err(error),
                },
                Some(had) if had.target == target => Ok(Named::Same),
                Some(had) => {
                    if pinned {
                        self.conflicts.push(format!(
                            "tag `{name}` points elsewhere in the store, and a bookmark on \
                             a revision is pinned; it was not moved"
                        ));
                        continue;
                    }
                    let Some(standing) = current_revision(&history, &had.target) else {
                        self.conflicts.push(format!(
                            "branch `{name}` has a bookmark whose change is not resolved \
                             in the store; it was not moved"
                        ));
                        continue;
                    };
                    match self.may_follow(&history, &reference, standing, revision) {
                        Ok(()) => store.set_name(&name, target).map(|()| Named::Moved),
                        Err(because) => {
                            self.conflicts
                                .push(format!("branch `{name}` was not moved: {because}"));
                            continue;
                        }
                    }
                }
            };

            if let (Ok(_), Some(oid)) = (&outcome, self.ref_oids.get(&reference)) {
                self.agreed.insert(reference.clone(), oid.clone());
            }
            match outcome {
                Ok(Named::New) => self.report.bookmarks.push(name),
                Ok(Named::Moved) => self.report.moved.push(name),
                Ok(Named::Same) => {}
                // Historica decides what a bookmark may be called and this
                // reports what it decided, rather than keeping a second copy of
                // the rule that would drift from it.
                Err(StoreError::UnusableName { because, .. }) => {
                    self.unnameable.push((reference, because))
                }
                // A branch called exactly what a change ID looks like. Rare to
                // the point of contrivance, and still a ref rather than a
                // conversion that should stop: decision 0024 refuses the name
                // because it would shadow the identifier it is spelled like.
                Err(StoreError::NameIsAnIdentifier { .. }) => self.unnameable.push((
                    reference,
                    "it is spelled as an identifier, and a bookmark that is one \
                     would stop that identifier naming its own file"
                        .to_owned(),
                )),
                Err(other) => return Err(other.into()),
            }
        }

        // A branch the last write left and git no longer has was deleted in
        // git. The bookmark stays, because historica's published API has no
        // way to remove one — which is a fact to take upstream rather than a
        // file to delete from here — and the report says so.
        for (reference, left) in self.known.refs.iter() {
            let Some(branch) = reference.strip_prefix("refs/heads/") else {
                continue;
            };
            // Still there, wherever it points: not this loop's concern.
            if self.ref_oids.contains_key(reference) {
                continue;
            }
            let Some(had) = store.bookmark(branch) else {
                continue;
            };
            let still_there = current_revision(&history, &had.target)
                .and_then(|standing| self.resolve(&left.oid).map(|was| was == standing))
                .unwrap_or(false);
            self.conflicts.push(if still_there {
                format!(
                    "branch `{branch}` was deleted in git; the bookmark stays where the \
                     branch was, because historica's API has no way to remove one"
                )
            } else {
                format!(
                    "branch `{branch}` was deleted in git after the store moved its \
                     bookmark; the bookmark stays where the store put it"
                )
            });
        }
        Ok(())
    }

    /// Whether a bookmark standing at `standing` may follow its ref to
    /// `arriving`, or why not.
    fn may_follow(
        &self,
        history: &History,
        reference: &str,
        standing: RevisionId,
        arriving: RevisionId,
    ) -> Result<(), String> {
        // A remembered commit that resolves to no revision — the record names
        // a commit neither side can place — is a record that answers nothing,
        // and is treated as absent rather than read as both sides having moved.
        match self
            .known
            .refs
            .left(reference)
            .and_then(|left| self.resolve(left))
        {
            Some(left) => {
                let store_moved = left != standing;
                let git_moved = left != arriving;
                match (git_moved, store_moved) {
                    (true, false) => Ok(()),
                    (false, _) => Err(
                        "git has it where the last write left it, and the store moved \
                         it since; `write` is what carries that across"
                            .to_owned(),
                    ),
                    (true, true) => Err(
                        "git and the store both moved it since the last write; point one \
                         at the other and run this again"
                            .to_owned(),
                    ),
                }
            }
            None => {
                if descends(history, arriving, standing) {
                    Ok(())
                } else {
                    Err(
                        "where git has it is not a step forward from where the store \
                         has it, and nothing remembers which side moved"
                            .to_owned(),
                    )
                }
            }
        }
    }

    /// The revisions this commit stands on, in the order git states them.
    fn parents(&self, commit: &Commit) -> Result<Vec<RevisionId>, Error> {
        let mut parents = Vec::new();
        for named in commit.from.iter().chain(&commit.merges) {
            let revision = match named {
                DataRef::Mark(mark) => *self
                    .revisions
                    .get(mark)
                    .ok_or(Error::ParentNotConverted { mark: *mark })?,
                // Named rather than carried: a commit the export left out
                // because the store already holds it.
                DataRef::Oid(oid) => self.named(oid)?,
            };
            // An empty commit maps to its own parent, so a merge of a branch
            // that ended in one can name the same revision twice. historica
            // takes a set of parents, not a list.
            if !parents.contains(&revision) {
                parents.push(revision);
            }
        }
        Ok(parents)
    }

    /// The revision a commit the export left out already is.
    ///
    /// Usually the record or the store answers at once. The exception is a
    /// commit that changed nothing: it was folded into its parent when it was
    /// read and kept out of the record, so a new branch grown from it names a
    /// commit neither side can place. Its tree is its parent's, so its
    /// revision is its parent's too, and git is asked for the parent — up the
    /// first-parent line, as many times as it takes.
    fn named(&self, oid: &str) -> Result<RevisionId, Error> {
        let mut at = oid.to_owned();
        let mut steps = 0;
        loop {
            if let Some(revision) = self.known.revision_of(&at) {
                return Ok(revision);
            }
            let Some(repository) = self.repository else {
                return Err(Error::ParentNotHeld { oid: at });
            };
            let parents = repository.parents_of(&at)?;
            let Some(parent) = parents.into_iter().next() else {
                return Err(Error::ParentNotHeld {
                    oid: oid.to_owned(),
                });
            };
            at = parent;
            steps += 1;
            if steps > 10_000 {
                return Err(Error::ParentNotHeld {
                    oid: oid.to_owned(),
                });
            }
        }
    }

    /// Renames, as historica wants them: one file at a time.
    ///
    /// Git has no directories, so `R sub other` moves everything beneath `sub`,
    /// and historica's `moves` names one file per entry — so the rename is
    /// expanded against the tree the commit started from.
    fn moves(&self, commit: &Commit, parent: &Tree) -> Result<Vec<(String, String)>, Error> {
        let mut moves = Vec::new();
        for change in &commit.changes {
            let Change::Rename {
                source,
                destination,
            } = change
            else {
                continue;
            };
            for (path, _) in parent.iter() {
                if let Some(rest) = under(path, source) {
                    let mut moved = destination.clone();
                    moved.extend_from_slice(rest);
                    moves.push((folder::path_of(path)?, folder::path_of(&moved)?));
                }
            }
        }
        Ok(moves)
    }

    fn finish(mut self) -> Report {
        if self.committers > 0 {
            let committers = self.committers;
            self.report.note(if committers == 1 {
                "one commit names a committer other than its author; historica \
                 records one person, so the author is the revision's and the \
                 committer is a `git.committer` header beside it"
                    .to_owned()
            } else {
                format!(
                    "{committers} commits name a committer other than their author; \
                     historica records one person, so the author is the revision's \
                     and the committer is a `git.committer` header beside it"
                )
            });
        }
        if self.encodings > 0 {
            let encodings = self.encodings;
            self.report.note(if encodings == 1 {
                "one commit stated a message encoding; the message was re-encoded \
                 to UTF-8 on the way out and the declaration is gone"
                    .to_owned()
            } else {
                format!(
                    "{encodings} commits stated a message encoding; the messages \
                     were re-encoded to UTF-8 on the way out and the declarations \
                     are gone"
                )
            });
        }
        if self.tags > 0 {
            let tags = self.tags;
            self.report.note(if tags == 1 {
                "one annotated tag did not cross; it is an object with a tagger \
                 and a message of its own, and historica has nowhere for either — \
                 a lightweight tag, which is only a pointer, crosses as a bookmark"
                    .to_owned()
            } else {
                format!(
                    "{tags} annotated tags did not cross; each is an object with a \
                     tagger and a message of its own, and historica has nowhere for \
                     either — a lightweight tag, which is only a pointer, crosses \
                     as a bookmark"
                )
            });
        }
        // Historica says which rule the name broke, and this repeats what it
        // said rather than paraphrasing it: a second copy of the rule here
        // would drift from the one that is actually enforced.
        for (reference, because) in std::mem::take(&mut self.unnameable) {
            self.report.note(format!(
                "`{reference}` did not cross: {because} — and a name this tool \
                 spelled some other way would be a name nobody could type"
            ));
        }
        for conflict in std::mem::take(&mut self.conflicts) {
            self.report.note(conflict);
        }
        if !self.elsewhere.is_empty() {
            let elsewhere = self
                .elsewhere
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            self.report.note(format!(
                "refs under {elsewhere} did not cross; each is a fact about \
                 somewhere else, and every commit they named is in the store"
            ));
        }
        if self.dangling > 0 {
            let dangling = self.dangling;
            self.report.note(format!(
                "{dangling} refs named something this stream did not carry, and \
                 nothing names them now"
            ));
        }
        if self.report.empty > 0 {
            let empty = self.report.empty;
            self.report.note(if empty == 1 {
                "one commit changed nothing and was not a merge; historica writes \
                 no revision for one, and what stood on it now stands on what it \
                 stood on"
                    .to_owned()
            } else {
                format!(
                    "{empty} commits changed nothing and were not merges; historica \
                     writes no revision for one, and what stood on them now stands \
                     on what they stood on"
                )
            });
        }
        self.report
    }
}

/// What naming a bookmark did.
enum Named {
    New,
    Moved,
    Same,
}

/// The revision a bookmark stands at now, for the one kind that can move.
fn current_revision(history: &History, target: &Name) -> Option<RevisionId> {
    match target {
        Name::Change(change) => match history.change_state(change) {
            ChangeState::Resolved(revision) => Some(revision.id),
            _ => None,
        },
        Name::Revision(revision) => Some(*revision),
        Name::File(_) => None,
    }
}

/// Whether `ancestor` is `descendant` or lies on some path of parents above it.
fn descends(history: &History, descendant: RevisionId, ancestor: RevisionId) -> bool {
    let mut seen = BTreeSet::new();
    let mut pending = vec![descendant];
    while let Some(at) = pending.pop() {
        if at == ancestor {
            return true;
        }
        if !seen.insert(at) {
            continue;
        }
        if let Some(revision) = history.get(&at) {
            pending.extend(revision.parents.iter().copied());
        }
    }
    false
}

/// The directory a ref that is neither a branch nor a tag sits in, so that a
/// hundred remote-tracking refs are reported as `refs/remotes/` once.
fn elsewhere(reference: &str) -> String {
    match reference.rfind('/') {
        Some(at) => reference[..=at].to_owned(),
        None => reference.to_owned(),
    }
}

/// `Name <email>`, which is how both git and historica spell a person.
fn spell(person: &Person) -> String {
    format!(
        "{} <{}>",
        String::from_utf8_lossy(&person.name),
        String::from_utf8_lossy(&person.email)
    )
}

/// The stream's seconds-and-offset, as historica's timestamp.
///
/// Formatted in the offset the commit carried rather than in the converter's
/// own, because "when this was written, where it was written" is the fact git
/// recorded and resolving it to UTC would throw half of it away.
fn when(person: &Person) -> Result<Timestamp, Error> {
    let offset =
        jiff::tz::Offset::from_seconds(person.offset_minutes * 60).map_err(|_| Error::Time {
            seconds: person.seconds,
        })?;
    let moment = jiff::Timestamp::from_second(person.seconds).map_err(|_| Error::Time {
        seconds: person.seconds,
    })?;
    let spelled = moment
        .to_zoned(offset.to_time_zone())
        .strftime("%Y-%m-%dT%H:%M:%S%:z")
        .to_string();
    spelled.parse().map_err(|_| Error::Time {
        seconds: person.seconds,
    })
}

/// What is left of `path` after `prefix`, if `path` is at or beneath it.
fn under<'a>(path: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    if path == prefix {
        return Some(&[]);
    }
    if path.len() > prefix.len() && path.starts_with(prefix) && path[prefix.len()] == b'/' {
        return Some(&path[prefix.len()..]);
    }
    None
}
