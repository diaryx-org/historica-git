//! The tree each commit leaves behind.
//!
//! `git fast-export` states a commit as a change list against its first parent,
//! not as a snapshot. That is the shape historica records in too, which is why
//! decision 0002 prefers it — but a conversion still has to know the whole file
//! set at a commit, because [`historica::record`] reads a folder rather than a
//! patch, and a folder has to be put there first.
//!
//! So this replays the stream: blobs are remembered, commits are applied to
//! their parent's tree, and what comes out is the tree as of each commit.
//!
//! **What this costs.** Every blob the stream carries is held in memory, shared
//! between the trees that name it, and a tree is kept for every commit that a
//! later commit might name as a parent — which, in one pass, is all of them.
//! That is fine for the repositories a person converts by hand and wrong for a
//! large one; the fix is to spool blobs to disk and to drop a tree once nothing
//! can reach it, and neither is done here.

use std::collections::{BTreeMap, HashMap};
use std::error;
use std::fmt;
use std::rc::Rc;

use crate::stream::{Blob, Change, Command, Commit, Content, DataRef, Mark, Mode, Reset};

/// What sits at a path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// The mode git states for it.
    pub mode: Mode,
    /// What it holds.
    pub content: Held,
}

/// The content of an entry, which is not always bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Held {
    /// A file's bytes, or a link's target. Shared between every tree that names
    /// this blob, because a file that survives a thousand commits is one file.
    Bytes(Rc<[u8]>),
    /// A submodule: the commit of another repository, which no blob carries and
    /// which a conversion has to decide about rather than write out.
    Submodule(String),
}

/// Every file at one commit, by path.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Tree {
    entries: BTreeMap<Vec<u8>, Entry>,
}

impl Tree {
    /// What is at this path, if anything.
    pub fn get(&self, path: &[u8]) -> Option<&Entry> {
        self.entries.get(path)
    }

    /// Every path and what is at it, in path order.
    pub fn iter(&self) -> impl Iterator<Item = (&Vec<u8>, &Entry)> {
        self.entries.iter()
    }

    /// How many files.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the commit left an empty folder.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Put an entry at a path, for a tree built from something other than a
    /// stream — git's own answer to `ls-tree`, when a commit's parent was left
    /// out of the export.
    pub fn insert(&mut self, path: Vec<u8>, entry: Entry) {
        self.entries.insert(path, entry);
    }

    /// Every path at or beneath `prefix`, which is what git means by naming a
    /// directory in a `D`, `R`, or `C`: git has no directories of its own, only
    /// paths that share a beginning.
    fn beneath(&self, prefix: &[u8]) -> Vec<Vec<u8>> {
        self.entries
            .keys()
            .filter(|path| at_or_beneath(path, prefix))
            .cloned()
            .collect()
    }
}

/// Replays a stream into the tree at each commit.
pub struct Trees {
    blobs: HashMap<Mark, Rc<[u8]>>,
    /// Trees by the mark of the commit that left them.
    marked: HashMap<Mark, Rc<Tree>>,
    /// Where each ref stands, so that a commit which states no `from` continues
    /// the branch it is committed to, as fast-import says it does.
    references: HashMap<Vec<u8>, Rc<Tree>>,
    /// Trees by the object ID of the commit that left them, for the commits an
    /// export named rather than carried. Nothing arrives here from the stream;
    /// a caller that can ask git seeds it.
    seeded: HashMap<String, Rc<Tree>>,
}

impl Default for Trees {
    fn default() -> Self {
        Self::new()
    }
}

impl Trees {
    /// A replay that has seen nothing.
    pub fn new() -> Self {
        Trees {
            blobs: HashMap::new(),
            marked: HashMap::new(),
            references: HashMap::new(),
            seeded: HashMap::new(),
        }
    }

    /// Tell the replay what a commit the stream only names left behind.
    ///
    /// `git fast-export --reference-excluded-parents` states a commit whose
    /// parent was excluded as a difference against that parent, named by
    /// object ID. The stream cannot carry that tree; whoever can ask git for
    /// it hands it in here before the commit that needs it arrives.
    pub fn seed(&mut self, oid: String, tree: Tree) {
        self.seeded.insert(oid, Rc::new(tree));
    }

    /// Whether a commit the stream names by object ID has been seeded.
    pub fn is_seeded(&self, oid: &str) -> bool {
        self.seeded.contains_key(oid)
    }

    /// The tree a commit starts from: its `from`, or the ref it continues.
    ///
    /// What a `R` or `D` in the commit is applied to, and so the tree a
    /// directory rename has to be expanded against. Ask *before* applying the
    /// commit: applying it moves the ref, and a commit that continues its ref
    /// would then be answered with its own tree.
    pub fn parent_of(&self, commit: &Commit) -> Result<Rc<Tree>, Error> {
        match &commit.from {
            Some(from) => self.tree_of(from),
            None => Ok(self
                .references
                .get(&commit.reference)
                .map(Rc::clone)
                .unwrap_or_default()),
        }
    }

    /// Hand the replay the next command.
    ///
    /// Returns the tree a commit left, and nothing for every other command —
    /// blobs and resets change what later commits mean without being a commit
    /// themselves.
    pub fn apply(&mut self, command: &Command) -> Result<Option<Rc<Tree>>, Error> {
        match command {
            Command::Blob(blob) => {
                self.blob(blob);
                Ok(None)
            }
            Command::Commit(commit) => self.commit(commit).map(Some),
            Command::Reset(reset) => {
                self.reset(reset)?;
                Ok(None)
            }
            // A tag names a commit; it does not change one.
            Command::Tag(_)
            | Command::Progress(_)
            | Command::Feature(_)
            | Command::Option(_)
            | Command::Done => Ok(None),
        }
    }

    fn blob(&mut self, blob: &Blob) {
        if let Some(mark) = blob.mark {
            self.blobs.insert(mark, Rc::from(blob.data.as_slice()));
        }
        // A blob with no mark can never be named again, so there is nothing to
        // remember about it.
    }

    fn reset(&mut self, reset: &Reset) -> Result<(), Error> {
        match &reset.from {
            Some(from) => {
                let tree = self.tree_of(from)?;
                self.references.insert(reset.reference.clone(), tree);
            }
            None => {
                self.references.remove(&reset.reference);
            }
        }
        Ok(())
    }

    fn commit(&mut self, commit: &Commit) -> Result<Rc<Tree>, Error> {
        let mut tree = match &commit.from {
            Some(from) => (*self.tree_of(from)?).clone(),
            // No `from` continues the ref, which is how fast-import reads a
            // stream that states a parent once and then keeps committing.
            None => self
                .references
                .get(&commit.reference)
                .map(|tree| (**tree).clone())
                .unwrap_or_default(),
        };

        for change in &commit.changes {
            self.change(&mut tree, change)?;
        }

        let tree = Rc::new(tree);
        if let Some(mark) = commit.mark {
            self.marked.insert(mark, Rc::clone(&tree));
        }
        self.references
            .insert(commit.reference.clone(), Rc::clone(&tree));
        Ok(tree)
    }

    fn change(&self, tree: &mut Tree, change: &Change) -> Result<(), Error> {
        match change {
            Change::Modify {
                mode,
                content,
                path,
            } => {
                let held = match mode {
                    Mode::Gitlink => match content {
                        Content::Named(DataRef::Oid(oid)) => Held::Submodule(oid.clone()),
                        _ => {
                            return Err(Error::SubmoduleWithoutACommit { path: shown(path) });
                        }
                    },
                    Mode::Directory => {
                        return Err(Error::WholeTree { path: shown(path) });
                    }
                    _ => Held::Bytes(self.content(content, path)?),
                };
                tree.entries.insert(
                    path.clone(),
                    Entry {
                        mode: *mode,
                        content: held,
                    },
                );
            }
            Change::Delete { path } => {
                for held in tree.beneath(path) {
                    tree.entries.remove(&held);
                }
            }
            Change::Rename {
                source,
                destination,
            } => {
                for path in tree.beneath(source) {
                    let entry = tree.entries.remove(&path).expect("just listed");
                    tree.entries
                        .insert(rebase(&path, source, destination), entry);
                }
            }
            Change::Copy {
                source,
                destination,
            } => {
                for path in tree.beneath(source) {
                    let entry = tree.entries.get(&path).expect("just listed").clone();
                    tree.entries
                        .insert(rebase(&path, source, destination), entry);
                }
            }
            Change::DeleteAll => tree.entries.clear(),
            // A note is filed against a commit rather than into the tree. A
            // conversion that carries notes reads them from the stream; this
            // replay has nothing to do with them.
            Change::Note { .. } => {}
        }
        Ok(())
    }

    fn content(&self, content: &Content, path: &[u8]) -> Result<Rc<[u8]>, Error> {
        match content {
            Content::Inline(bytes) => Ok(Rc::from(bytes.as_slice())),
            Content::Named(DataRef::Mark(mark)) => {
                self.blobs
                    .get(mark)
                    .map(Rc::clone)
                    .ok_or(Error::NoSuchBlob {
                        mark: *mark,
                        path: shown(path),
                    })
            }
            Content::Named(DataRef::Oid(oid)) => Err(Error::ContentNotCarried {
                oid: oid.clone(),
                path: shown(path),
            }),
        }
    }

    fn tree_of(&self, reference: &DataRef) -> Result<Rc<Tree>, Error> {
        match reference {
            DataRef::Mark(mark) => self
                .marked
                .get(mark)
                .map(Rc::clone)
                .ok_or(Error::NoSuchCommit { mark: *mark }),
            DataRef::Oid(oid) => self
                .seeded
                .get(oid)
                .map(Rc::clone)
                .ok_or_else(|| Error::CommitNotCarried { oid: oid.clone() }),
        }
    }
}

/// A stream that read correctly but does not describe a history this can
/// replay.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Error {
    /// A change named a blob the stream had not carried.
    NoSuchBlob {
        /// The mark it named.
        mark: Mark,
        /// Where the content was going.
        path: String,
    },
    /// A commit named a parent the stream had not carried.
    NoSuchCommit {
        /// The mark it named.
        mark: Mark,
    },
    /// The stream named content by object ID, which means it was written
    /// without the content in it.
    ContentNotCarried {
        /// The object ID named.
        oid: String,
        /// Where the content was going.
        path: String,
    },
    /// The stream named a commit by object ID rather than carrying it.
    CommitNotCarried {
        /// The object ID named.
        oid: String,
    },
    /// A `040000` entry: the stream states whole subtrees rather than files.
    WholeTree {
        /// The directory named.
        path: String,
    },
    /// A `160000` entry that named no commit of the other repository.
    SubmoduleWithoutACommit {
        /// Where the submodule sits.
        path: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoSuchBlob { mark, path } => write!(
                f,
                "`{path}` is content the stream never carried: mark :{} is named \
                 before it is set",
                mark.0
            ),
            Error::NoSuchCommit { mark } => write!(
                f,
                "a commit names :{} as a parent, which the stream never set",
                mark.0
            ),
            Error::ContentNotCarried { oid, path } => write!(
                f,
                "`{path}` names content by object ID ({oid}) — the export was \
                 written with `--no-data`, and a conversion needs the content"
            ),
            Error::CommitNotCarried { oid } => write!(
                f,
                "a commit names parent {oid} by object ID — the export covers \
                 part of a history, and a conversion needs the whole of it"
            ),
            Error::WholeTree { path } => write!(
                f,
                "`{path}` is a whole subtree — the export was written with \
                 `--full-tree`, and a conversion reads the ordinary form"
            ),
            Error::SubmoduleWithoutACommit { path } => write!(
                f,
                "the submodule at `{path}` names no commit of its own repository"
            ),
        }
    }
}

impl error::Error for Error {}

/// A path git states, for a message a person reads. Git's paths are bytes and
/// almost all of them are UTF-8; one that is not is shown rather than refused,
/// because this is an error message and not a decision about the path.
fn shown(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

/// Is `path` `prefix`, or under it? `sub/a` is under `sub`; `subtle` is not.
fn at_or_beneath(path: &[u8], prefix: &[u8]) -> bool {
    path == prefix
        || (path.len() > prefix.len() && path.starts_with(prefix) && path[prefix.len()] == b'/')
}

/// `sub/a`, moved from `sub` to `other`, is `other/a`.
fn rebase(path: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut moved = to.to_vec();
    moved.extend_from_slice(&path[from.len()..]);
    moved
}
