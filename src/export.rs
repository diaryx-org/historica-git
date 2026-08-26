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
//! commits, and the commit-to-revision correspondence needs filing nowhere. Ask
//! `git fast-import` for a marks file when you want it.
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
    /// Refs the conversion moved.
    pub references: Vec<String>,
    /// The branch the conversion would have a fresh repository check out.
    pub branch: Option<String>,
    /// One line per kind of fact that did not cross, for a person to read.
    pub uncarried: Vec<String>,
}

impl Report {
    fn note(&mut self, line: String) {
        self.uncarried.push(line);
    }
}

/// Convert the store under `folder` into a git repository at `repository`.
///
/// Runs `git init` and then `git fast-import`. The repository must be empty or
/// absent, for the reason import's target must be: a conversion only writes
/// where it can be sure it owns everything it touches.
pub fn to_repository(folder: &Path, repository: &Path) -> Result<Report, Error> {
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

    let mut child = Process::new("git")
        .arg("-C")
        .arg(repository)
        .args(["fast-import", "--quiet", "--force"])
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
    let outcome = to_stream(folder, stream);

    let said = listening.join().unwrap_or_default();
    let status = child.wait().map_err(Error::Spawn)?;
    let mut report = outcome?;
    if !status.success() {
        return Err(Error::GitFailed {
            status: status.code(),
            said,
        });
    }
    check_out(repository, &mut report)?;
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
fn check_out(repository: &Path, report: &mut Report) -> Result<(), Error> {
    let Some(branch) = report.branch.clone() else {
        report.note(
            "nothing says which branch is checked out, because no bookmark named a \
             branch; git's own default is what HEAD points at and it points at \
             nothing"
                .to_owned(),
        );
        return Ok(());
    };

    for arguments in [
        vec![
            "symbolic-ref".to_owned(),
            "HEAD".to_owned(),
            format!("{HEADS}{branch}"),
        ],
        vec![
            "reset".to_owned(),
            "--quiet".to_owned(),
            "--hard".to_owned(),
        ],
    ] {
        let status = Process::new("git")
            .arg("-C")
            .arg(repository)
            .args(&arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(Error::Spawn)?;
        if !status.success() {
            return Err(Error::GitFailed {
                status: status.code(),
                said: vec![format!("`git {}` failed", arguments.join(" "))],
            });
        }
    }
    Ok(())
}

/// The same conversion, written wherever the caller wants the bytes.
///
/// This is where the work is, and it needs no git — which is what makes the
/// whole of it testable, exactly as `import::from_stream` is.
pub fn to_stream<W: Write>(folder: &Path, out: W) -> Result<Report, Error> {
    let store = Store::open(folder.join(STORE_DIR))?;
    let mut conversion = Conversion {
        writer: Writer::new(out),
        marks: BTreeMap::new(),
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
    };
    conversion.run(&store)?;
    let branch = checked_out(&conversion.branches);
    let mut report = conversion.report;
    report.branch = branch;
    Ok(report)
}

/// One conversion in progress.
struct Conversion<W> {
    writer: Writer<W>,
    /// Where each revision landed in the stream.
    marks: BTreeMap<RevisionId, Mark>,
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
}

impl<W: Write> Conversion<W> {
    fn run(&mut self, store: &Store) -> Result<(), Error> {
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

        for id in order(&history, &visible) {
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
                from: parents.first().map(|mark| DataRef::Mark(*mark)),
                merges: parents.iter().skip(1).map(|m| DataRef::Mark(*m)).collect(),
                changes,
            }))?;
            self.marks.insert(id, mark);
            self.report.commits += 1;
        }

        self.refs(store, &history, &visible)?;
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

    /// The marks this revision's commit stands on.
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
    ) -> Vec<Mark> {
        let mut marks = Vec::new();
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
            if let Some(mark) = self.marks.get(&standing)
                && !marks.contains(mark)
            {
                marks.push(*mark);
            }
        }
        marks
    }

    /// Move the refs, then take the staging ref away.
    fn refs(
        &mut self,
        store: &Store,
        history: &historica::core::History,
        visible: &BTreeSet<RevisionId>,
    ) -> Result<(), Error> {
        let mut named: BTreeSet<RevisionId> = BTreeSet::new();

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
            let Some(mark) = self.marks.get(&target).copied() else {
                continue;
            };
            let Some(reference) = git_ref(directory, name) else {
                self.report.note(format!(
                    "the bookmark `{name}` did not cross; git has no ref by that name"
                ));
                continue;
            };
            if directory == HEADS {
                self.branches.push(name.clone());
            }
            self.write(&Command::Reset(Reset {
                reference: reference.clone().into_bytes(),
                from: Some(DataRef::Mark(mark)),
            }))?;
            self.report.references.push(reference);
            named.insert(target);
        }

        // A head nobody bookmarked is still work somebody did, and a commit no
        // ref reaches is a commit git will collect. So it goes somewhere out of
        // the way rather than nowhere.
        for head in history.heads() {
            if !visible.contains(&head) || named.contains(&head) {
                continue;
            }
            let Some(mark) = self.marks.get(&head).copied() else {
                continue;
            };
            let reference = format!("{UNNAMED}/{}", head.abbreviate(12));
            self.write(&Command::Reset(Reset {
                reference: reference.clone().into_bytes(),
                from: Some(DataRef::Mark(mark)),
            }))?;
            self.report.references.push(reference);
        }

        // Take the staging ref away, which fast-import spells as a reset to the
        // null object ID.
        self.write(&Command::Reset(Reset {
            reference: STAGING.as_bytes().to_vec(),
            from: Some(DataRef::Oid("0".repeat(40))),
        }))
    }

    fn mark(&mut self) -> Mark {
        self.next += 1;
        Mark(self.next)
    }

    fn write(&mut self, command: &Command) -> Result<(), Error> {
        self.writer.write(command).map_err(Error::Write)
    }

    fn finish(&mut self) {
        let uncrossed = self.report.revisions - self.report.commits;
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
        self.report.note(
            "change IDs did not cross; git has nowhere to put one, and decision 0003 \
             derives one from a commit rather than the other way about"
                .to_owned(),
        );
    }
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
