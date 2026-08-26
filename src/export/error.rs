//! Why a store could not be written out.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use historica::core::RevisionId;

/// What stopped a conversion.
#[derive(Debug)]
pub enum Error {
    /// The store would not open.
    Store(historica::store::StoreError),
    /// A revision's tree or content could not be materialised.
    Materialise(Box<historica::store::MaterialiseError>),
    /// Writing the stream failed.
    Write(io::Error),
    /// A path could not be read or written.
    Io {
        /// Where.
        at: PathBuf,
        /// What the operating system said.
        because: io::Error,
    },
    /// `git` could not be started.
    Spawn(io::Error),
    /// `git fast-import` refused the stream.
    GitFailed {
        /// The exit status, where there was one.
        status: Option<i32>,
        /// What git said on the way out.
        said: Vec<String>,
    },
    /// The target is not somewhere a conversion may write.
    NotFree {
        /// Where.
        at: PathBuf,
        /// What is in the way.
        because: String,
    },
    /// The history names a revision whose document this store does not hold.
    Missing {
        /// The revision.
        revision: RevisionId,
    },
    /// A revision names a `when` this crate cannot turn into git's two numbers.
    Time {
        /// The revision.
        revision: RevisionId,
        /// What it said.
        when: String,
    },
    /// Two branches each stated a file's whole bytes, and historica's decision
    /// 0008 declines to choose. Neither does this.
    Contested {
        /// The revision reached.
        revision: RevisionId,
        /// Where the file sits.
        path: String,
    },
    /// A file's bytes are not in this store, so the revision cannot be written
    /// whole.
    Absent {
        /// The revision reached.
        revision: RevisionId,
        /// Where the file sits.
        path: String,
    },
}

impl Error {
    pub(crate) fn io(at: &Path, because: io::Error) -> Self {
        Error::Io {
            at: at.to_path_buf(),
            because,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Store(error) => write!(f, "{error}"),
            Error::Materialise(error) => write!(f, "{error}"),
            Error::Write(error) => write!(f, "writing the stream failed: {error}"),
            Error::Io { at, because } => write!(f, "{}: {because}", at.display()),
            Error::Spawn(error) => write!(
                f,
                "`git` could not be started: {error}\n\ndecision 0002 makes git \
                 this tool's one dependency, and it must be on PATH"
            ),
            Error::GitFailed { status, said } => {
                match status {
                    Some(code) => write!(f, "`git fast-import` exited {code}")?,
                    None => write!(f, "`git fast-import` was killed")?,
                }
                for line in said {
                    write!(f, "\n  {line}")?;
                }
                Ok(())
            }
            Error::NotFree { at, because } => write!(
                f,
                "{}: {because}\n\na conversion only writes where it can be sure it \
                 owns everything it removes",
                at.display()
            ),
            Error::Missing { revision } => write!(
                f,
                "the history names revision {}, and this store does not hold its \
                 document\n\nthe store is incomplete; `historica check` says what is \
                 missing and `fetch` is how it arrives",
                revision.abbreviate(12)
            ),
            Error::Time { revision, when } => write!(
                f,
                "revision {} says it was written at `{when}`, which is not a moment \
                 git can be told about",
                revision.abbreviate(12)
            ),
            Error::Contested { revision, path } => write!(
                f,
                "at revision {}, two branches each stated the whole bytes of \
                 `{path}`, and historica's decision 0008 declines to choose between \
                 them\n\ngit has no way to say a file has no content, and writing \
                 either side would be inventing the answer historica refused to \
                 invent — record which one is right, then convert",
                revision.abbreviate(12)
            ),
            Error::Absent { revision, path } => write!(
                f,
                "at revision {}, the bytes of `{path}` are not in this store, so the \
                 revision cannot be written whole\n\nthey were either forgotten or \
                 never fetched; a git commit has no absent blob",
                revision.abbreviate(12)
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<historica::store::StoreError> for Error {
    fn from(error: historica::store::StoreError) -> Self {
        Error::Store(error)
    }
}

impl From<historica::store::MaterialiseError> for Error {
    fn from(error: historica::store::MaterialiseError) -> Self {
        Error::Materialise(Box::new(error))
    }
}
