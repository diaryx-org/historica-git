//! What a repository remembers of the conversions that touched it.
//!
//! Decision 0007. Two files under `.git/historica/`, both plain text, both
//! checkable by hand, and neither in the store:
//!
//! - `commits` — one line per revision this tool has written as a commit,
//!   `<revision> <object ID>`. Decision 0004 makes this *derived*: a commit is
//!   a function of its revision, so the file can be deleted and a full write
//!   regenerates every line of it. What it saves is sending a commit git
//!   already has.
//! - `refs` — one line per ref the last conversion answers for, `<ref> <object
//!   ID>` where git and the store last agreed, with ` made` after it where a
//!   write of this tool's is what put the ref there. This is the one fact
//!   nothing else holds: whether a ref that now points elsewhere was moved by
//!   git or by this tool is only answerable by remembering where the two last
//!   agreed, and whether a ref is this tool's to delete is only answerable by
//!   remembering whether it made it.
//!
//! They live in the git directory because they are facts about *this
//! repository* — 0004 refuses to let a store carry a fact about a repository
//! somewhere else, and historica's 0053 gives a store directory only by
//! decision. `.git/` is where git's own tooling keeps a tool's private state
//! (`lfs/`, `info/`), and a repository is the one thing a conversion is
//! already allowed to write.

use std::collections::BTreeMap;
use std::error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use historica::core::RevisionId;

/// The directory under `.git/` these files sit in.
pub const DIRECTORY: &str = "historica";

/// The file naming the commit each written revision became.
pub const COMMITS: &str = "commits";

/// The file naming where the last write left each ref it moved.
pub const REFS: &str = "refs";

/// Which commit each revision became, and the other way about.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Commits {
    by_revision: BTreeMap<RevisionId, String>,
    by_oid: BTreeMap<String, RevisionId>,
}

impl Commits {
    /// Read the file at `git_dir/historica/commits`, or nothing if there is
    /// none — a repository this tool never wrote remembers nothing, which is
    /// the same as a file somebody deleted.
    pub fn read(git_dir: &Path) -> Result<Self, Error> {
        let mut commits = Commits::default();
        for (key, value) in lines(&git_dir.join(DIRECTORY).join(COMMITS))? {
            let revision: RevisionId = key.parse().map_err(|_| Error::Malformed {
                file: COMMITS,
                line: format!("{key} {value}"),
            })?;
            commits.insert(revision, value);
        }
        Ok(commits)
    }

    /// Write the file, whole.
    pub fn write(&self, git_dir: &Path) -> Result<(), Error> {
        write(
            &git_dir.join(DIRECTORY).join(COMMITS),
            self.by_revision
                .iter()
                .map(|(revision, oid)| format!("{revision} {oid}")),
        )
    }

    /// Remember that `revision` was written as `oid`.
    pub fn insert(&mut self, revision: RevisionId, oid: String) {
        self.by_oid.insert(oid.clone(), revision);
        self.by_revision.insert(revision, oid);
    }

    /// The commit a revision became, if it has been written.
    pub fn oid_of(&self, revision: &RevisionId) -> Option<&str> {
        self.by_revision.get(revision).map(String::as_str)
    }

    /// The revision a commit was written from, if this tool wrote it.
    pub fn revision_of(&self, oid: &str) -> Option<RevisionId> {
        self.by_oid.get(oid).copied()
    }

    /// Every object ID remembered.
    pub fn oids(&self) -> impl Iterator<Item = &str> {
        self.by_oid.keys().map(String::as_str)
    }

    /// Forget a commit the repository no longer holds.
    pub fn remove_oid(&mut self, oid: &str) {
        if let Some(revision) = self.by_oid.remove(oid) {
            self.by_revision.remove(&revision);
        }
    }

    /// How many are remembered.
    pub fn len(&self) -> usize {
        self.by_revision.len()
    }

    /// Whether nothing is.
    pub fn is_empty(&self) -> bool {
        self.by_revision.is_empty()
    }
}

/// Where a ref stood when git and the store last agreed about it, and whether
/// this tool is the one that put it there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Left {
    /// The commit the ref pointed at.
    pub oid: String,
    /// Whether a write of this tool's created or moved the ref — as opposed to
    /// an import finding git and the store already in agreement about a ref
    /// somebody else made. Only a ref this tool made is this tool's to delete.
    pub made: bool,
}

/// Where the last conversion left each ref it answers for.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Refs {
    /// Present only where a conversion has happened; absent means this tool
    /// has never touched a ref here, which is a different fact from having
    /// touched none.
    written: Option<BTreeMap<String, Left>>,
}

impl Refs {
    /// Read the file at `git_dir/historica/refs`, or note that there is none.
    pub fn read(git_dir: &Path) -> Result<Self, Error> {
        let path = git_dir.join(DIRECTORY).join(REFS);
        if !path.exists() {
            return Ok(Refs { written: None });
        }
        let mut written = BTreeMap::new();
        for (reference, value) in lines(&path)? {
            // `<oid>` or `<oid> made`.
            let (oid, made) = match value.split_once(' ') {
                None => (value, false),
                Some((oid, MADE)) => (oid.to_owned(), true),
                Some(_) => {
                    return Err(Error::Malformed {
                        file: REFS,
                        line: format!("{reference} {value}"),
                    });
                }
            };
            written.insert(reference, Left { oid, made });
        }
        Ok(Refs {
            written: Some(written),
        })
    }

    /// A repository nothing was ever written to.
    pub fn none() -> Self {
        Refs { written: None }
    }

    /// Whether any conversion has been remembered at all.
    pub fn known(&self) -> bool {
        self.written.is_some()
    }

    /// Where the last conversion left this ref, if it answers for it.
    pub fn left(&self, reference: &str) -> Option<&str> {
        self.entry(reference).map(|left| left.oid.as_str())
    }

    /// Whether a write of this tool's is what put the ref where it is.
    pub fn made(&self, reference: &str) -> bool {
        self.entry(reference).is_some_and(|left| left.made)
    }

    fn entry(&self, reference: &str) -> Option<&Left> {
        self.written.as_ref().and_then(|refs| refs.get(reference))
    }

    /// Every ref the last conversion answers for.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Left)> {
        self.written
            .iter()
            .flat_map(|refs| refs.iter().map(|(k, v)| (k.as_str(), v)))
    }

    /// The whole record, to be carried forward and amended.
    pub fn to_map(&self) -> BTreeMap<String, Left> {
        self.written.clone().unwrap_or_default()
    }

    /// Write the file, whole.
    pub fn write(git_dir: &Path, left: &BTreeMap<String, Left>) -> Result<(), Error> {
        write(
            &git_dir.join(DIRECTORY).join(REFS),
            left.iter().map(|(reference, left)| match left.made {
                true => format!("{reference} {} {MADE}", left.oid),
                false => format!("{reference} {}", left.oid),
            }),
        )
    }
}

/// The word on a `refs` line that says a write of this tool's put the ref
/// where it is.
const MADE: &str = "made";

fn lines(path: &Path) -> Result<Vec<(String, String)>, Error> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(path, error)),
    };
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // The key is the first word: a revision digest or a ref name, neither
        // of which holds a space. What follows is the line's to parse.
        let Some((key, value)) = line.split_once(' ') else {
            return Err(Error::Malformed {
                file: name_of(path),
                line: line.to_owned(),
            });
        };
        out.push((key.to_owned(), value.to_owned()));
    }
    Ok(out)
}

fn write(path: &Path, lines: impl Iterator<Item = String>) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    let mut text = String::new();
    for line in lines {
        text.push_str(&line);
        text.push('\n');
    }
    // Written beside and renamed over, so a reader never meets half a file.
    let staging = path.with_extension("tmp");
    fs::write(&staging, text).map_err(|error| Error::io(&staging, error))?;
    fs::rename(&staging, path).map_err(|error| Error::io(path, error))
}

fn name_of(path: &Path) -> &'static str {
    match path.file_name().and_then(|name| name.to_str()) {
        Some(REFS) => REFS,
        _ => COMMITS,
    }
}

/// A remembered file that could not be read or written.
#[derive(Debug)]
pub enum Error {
    /// The filesystem refused.
    Io {
        /// Which file.
        path: PathBuf,
        /// What went wrong.
        error: io::Error,
    },
    /// A line was not `<key> <value>`.
    Malformed {
        /// Which file.
        file: &'static str,
        /// The line.
        line: String,
    },
}

impl Error {
    fn io(path: &Path, error: io::Error) -> Self {
        Error::Io {
            path: path.to_path_buf(),
            error,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Error::Malformed { file, line } => {
                write!(
                    f,
                    "`.git/{DIRECTORY}/{file}` holds `{line}`, which is not a line this \
                     tool writes"
                )?;
                match *file {
                    COMMITS => write!(
                        f,
                        " — the file is derived, and deleting it costs a full \
                         conversion and nothing else"
                    ),
                    _ => write!(
                        f,
                        " — the file says where the last write left each ref, and \
                         without it the next write treats every ref git holds as \
                         somebody else's"
                    ),
                }
            }
        }
    }
}

impl error::Error for Error {}
