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
//! Nothing of a person's is ever deleted here. The target folder must be empty
//! or absent, and the only paths this removes are ones it wrote itself on a
//! previous commit.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::rc::Rc;

use historica::core::RevisionId;
use historica::format::Timestamp;
use historica::record::{self, Recording, Restriction};
use historica::store::{Bookmark, Name, STORE_DIR, Store, StoreError};
use historica::working::{Skipped, Working};

use crate::carried;
use crate::identity::Identity;
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
    /// Bookmarks named at a converted revision.
    pub bookmarks: Vec<String>,
    /// One line per kind of fact that did not cross, for a person to read.
    pub uncarried: Vec<String>,
}

impl Report {
    fn note(&mut self, line: String) {
        self.uncarried.push(line);
    }
}

/// Convert the repository at `repository` into a fresh store under `folder`.
///
/// Runs `git fast-export` and reads what it writes. The flags are decision
/// 0002's and 0003's: `-M` so a rename arrives as a rename,
/// `--show-original-ids` because a change ID is derived from the object ID, and
/// the two signature modes because git otherwise refuses outright at the first
/// signed tag.
pub fn from_repository(repository: &Path, folder: &Path) -> Result<Report, Error> {
    let mut child = Process::new("git")
        .arg("-C")
        .arg(repository)
        .args([
            "fast-export",
            "--all",
            "-M",
            "--show-original-ids",
            "--reencode=yes",
            // Decision 0004's measurement was that a signed commit does not
            // round-trip, and historica's 0070 opened the door that closes it:
            // the signature comes across verbatim and is recorded as a header
            // this tool owns. `strip` would throw away the one fact that makes
            // the commit what it is.
            "--signed-commits=verbatim",
            "--signed-tags=strip",
        ])
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

    let stream = child.stdout.take().expect("stdout was piped");
    let outcome = from_stream(BufReader::new(stream), folder);

    let said = listening.join().unwrap_or_default();
    let status = child.wait().map_err(Error::Spawn)?;
    let report = outcome?;
    if !status.success() {
        return Err(Error::GitFailed {
            status: status.code(),
            said,
        });
    }
    Ok(report)
}

/// The same conversion, from a stream somebody else produced.
///
/// This is where the work is, and it needs no git: a stream in a file converts
/// exactly as a stream from a pipe does, which is what makes the whole of it
/// testable against the corpus.
pub fn from_stream<R: BufRead>(input: R, folder: &Path) -> Result<Report, Error> {
    folder::must_be_free(folder)?;
    fs::create_dir_all(folder).map_err(|error| Error::io(folder, error))?;
    let mut store = Store::init(folder.join(STORE_DIR))?;

    let mut conversion = Conversion {
        folder: folder.to_path_buf(),
        trees: Trees::new(),
        written: Rc::new(Tree::default()),
        revisions: BTreeMap::new(),
        report: Report::default(),
        committers: 0,
        signatures: 0,
        encodings: 0,
        tags: 0,
        refs: BTreeMap::new(),
        unnameable: Vec::new(),
        elsewhere: BTreeSet::new(),
        dangling: 0,
    };

    for command in Reader::new(input) {
        let command = command?;
        let tree = conversion.trees.apply(&command)?;
        match (&command, tree) {
            (Command::Commit(commit), Some(tree)) => {
                conversion.commit(&mut store, commit, tree)?;
            }
            // An annotated tag is an object with a tagger and a message of its
            // own, and historica has nowhere for either. Its `from` is not
            // followed into a bookmark, because a bookmark would say the tag
            // crossed when what crossed is where it pointed.
            (Command::Tag(_), _) => conversion.tags += 1,
            (Command::Reset(reset), _) => conversion.moved(reset),
            _ => {}
        }
    }
    conversion.name(&mut store)?;
    Ok(conversion.finish())
}

/// One conversion in progress.
struct Conversion {
    folder: PathBuf,
    trees: Trees,
    /// What is on disk right now, so that materialising the next commit writes
    /// the difference rather than the whole tree.
    written: Rc<Tree>,
    /// Where each commit of the stream landed in the store.
    revisions: BTreeMap<Mark, RevisionId>,
    report: Report,
    committers: usize,
    signatures: usize,
    encodings: usize,
    tags: usize,
    /// Where each ref of the stream ended up, played forward exactly as the
    /// stream moves them: a `commit` moves the ref it names, and a `reset`
    /// moves the ref it names to wherever it says.
    refs: BTreeMap<String, RevisionId>,
    unnameable: Vec<String>,
    elsewhere: BTreeSet<String>,
    dangling: usize,
}

impl Conversion {
    fn commit(&mut self, store: &mut Store, commit: &Commit, tree: Rc<Tree>) -> Result<(), Error> {
        self.report.commits += 1;
        let mut identity = Identity::of(commit)?;

        // The folder, as this commit has it.
        let parent = Rc::clone(&self.written);
        folder::materialise(&self.folder, &parent, &tree)?;
        self.written = Rc::clone(&tree);

        let working = Working::read(&self.folder, &Skipped::none())?;
        let parents = self.parents(commit)?;
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

        let reference = String::from_utf8_lossy(&commit.reference).into_owned();
        match record::record(store, &working, &recording, &mut identity) {
            Ok(recorded) => {
                self.report.revisions += 1;
                self.refs.insert(reference, recorded.revision);
                if let Some(mark) = commit.mark {
                    self.revisions.insert(mark, recorded.revision);
                }
            }
            // A commit that changed nothing and joins nothing is not a
            // revision historica can write — `record` says so rather than
            // writing an empty one. Its descendants stand on its parent, so
            // the shape of the history is kept even though the commit is not.
            Err(record::RecordError::NothingToRecord) => {
                self.report.empty += 1;
                if let Some(parent) = recording.parents.first() {
                    self.refs.insert(reference, *parent);
                    if let Some(mark) = commit.mark {
                        self.revisions.insert(mark, *parent);
                    }
                }
            }
            Err(other) => return Err(Error::Record(Box::new(other))),
        }
        Ok(())
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

    /// Name a bookmark at each ref the stream left somewhere.
    ///
    /// Decision 0006: a branch follows the work, so it becomes a bookmark on
    /// the change; a tag names one version and does not move, so it becomes a
    /// bookmark on the revision. Historica's two kinds of target are the
    /// distinction git spells with two directories, which is what lets both
    /// cross without this inventing a grammar for either.
    fn name(&mut self, store: &mut Store) -> Result<(), Error> {
        for (reference, revision) in std::mem::take(&mut self.refs) {
            let target = if let Some(branch) = reference.strip_prefix("refs/heads/") {
                let Some(change) = store.revision(&revision).map(|revision| revision.change) else {
                    self.dangling += 1;
                    continue;
                };
                (branch.to_owned(), Name::Change(change))
            } else if let Some(tag) = reference.strip_prefix("refs/tags/") {
                (tag.to_owned(), Name::Revision(revision))
            } else {
                // `refs/remotes/`, `refs/notes/`, and whatever else a
                // repository keeps. Each is a fact about somewhere else, and a
                // bookmark that claimed otherwise would be this tool deciding
                // what somebody's remote-tracking ref meant.
                self.elsewhere.insert(elsewhere(&reference));
                continue;
            };

            let (name, target) = target;
            match store.set_bookmark(&name, Bookmark::shared(target)) {
                Ok(()) => self.report.bookmarks.push(name),
                // Historica decides what a bookmark may be called and this
                // reports what it decided, rather than keeping a second copy of
                // the rule that would drift from it.
                Err(StoreError::UnusableName { .. } | StoreError::NameIsAnIdentifier { .. }) => {
                    self.unnameable.push(reference)
                }
                Err(other) => return Err(other.into()),
            }
        }
        Ok(())
    }

    /// The revisions this commit stands on, in the order git states them.
    fn parents(&self, commit: &Commit) -> Result<Vec<RevisionId>, Error> {
        let mut parents = Vec::new();
        for named in commit.from.iter().chain(&commit.merges) {
            let DataRef::Mark(mark) = named else {
                return Err(Error::ParentNotCarried);
            };
            let revision = self
                .revisions
                .get(mark)
                .ok_or(Error::ParentNotConverted { mark: *mark })?;
            // An empty commit maps to its own parent, so a merge of a branch
            // that ended in one can name the same revision twice. historica
            // takes a set of parents, not a list.
            if !parents.contains(revision) {
                parents.push(*revision);
            }
        }
        Ok(parents)
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
        if !self.unnameable.is_empty() {
            let refused = self.unnameable.join(", ");
            self.report.note(format!(
                "these refs did not cross: {refused} — historica will not hold a \
                 bookmark by those names, and a name this tool spelled some other \
                 way would be a name nobody could type"
            ));
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
