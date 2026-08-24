//! What stops a conversion.

use std::error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::stream::Mark;
use crate::{identity, stream, tree};

/// A conversion that did not finish, and why.
#[derive(Debug)]
pub enum Error {
    /// The stream would not read.
    Stream(stream::Error),
    /// The stream read, but does not describe a history that can be replayed.
    Replay(tree::Error),
    /// A commit carried no object ID, or one this cannot use.
    Identity(identity::Error),
    /// The store refused.
    Store(Box<historica::store::StoreError>),
    /// The folder would not be read as a working copy.
    Working(Box<historica::working::WorkingError>),
    /// historica declined to record the revision.
    Record(Box<historica::record::RecordError>),
    /// The target folder holds something already.
    NotEmpty {
        /// Where the conversion was going.
        folder: PathBuf,
        /// One thing that is in the way.
        holding: String,
    },
    /// A path in the repository is not something a store can hold.
    UnusablePath {
        /// The path git stated.
        path: String,
        /// Why it cannot be used.
        because: String,
    },
    /// A path in the repository is not UTF-8.
    PathNotText {
        /// The path, as far as it can be shown.
        path: String,
    },
    /// A commit's parent was named by object ID rather than carried.
    ParentNotCarried,
    /// A commit's parent had not been converted when the commit arrived, which
    /// means the stream was not in the order git writes.
    ParentNotConverted {
        /// The mark the commit named.
        mark: Mark,
    },
    /// A commit states a moment that cannot be spelled as a timestamp.
    Time {
        /// The seconds the stream carried.
        seconds: i64,
    },
    /// `git` could not be run at all.
    Spawn(io::Error),
    /// `git fast-export` ran and failed.
    GitFailed {
        /// Its exit code, where it had one.
        status: Option<i32>,
        /// What it said on the way out.
        said: Vec<String>,
    },
    /// The filesystem refused.
    Io {
        /// What was being read or written.
        path: PathBuf,
        /// What went wrong.
        error: io::Error,
    },
    /// A link, on a platform that has none.
    #[cfg(not(unix))]
    NoLinks {
        /// Where the link would have gone.
        path: PathBuf,
    },
}

impl Error {
    pub(super) fn io(path: impl AsRef<Path>, error: io::Error) -> Self {
        Error::Io {
            path: path.as_ref().to_path_buf(),
            error,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Stream(error) => write!(f, "the export could not be read: {error}"),
            Error::Replay(error) => write!(f, "the export could not be replayed: {error}"),
            Error::Identity(error) => write!(f, "{error}"),
            Error::Store(error) => write!(f, "the store refused: {error}"),
            Error::Working(error) => write!(f, "the folder could not be read: {error}"),
            Error::Record(error) => write!(f, "the revision was not recorded: {error}"),
            Error::NotEmpty { folder, holding } => write!(
                f,
                "{} is not empty — it holds `{holding}` — and a conversion will \
                 only write into a folder it can be sure it owns",
                folder.display()
            ),
            Error::UnusablePath { path, because } => {
                write!(f, "the repository holds `{path}`, which {because}")
            }
            Error::PathNotText { path } => write!(
                f,
                "the repository holds `{path}`, whose name is not UTF-8, and a \
                 store spells its paths in text"
            ),
            Error::ParentNotCarried => write!(
                f,
                "a commit names a parent by object ID — the export covers part of \
                 a history, and a conversion needs the whole of it"
            ),
            Error::ParentNotConverted { mark } => write!(
                f,
                "a commit stands on :{}, which has not been converted — the export \
                 is not in the order git writes it in",
                mark.0
            ),
            Error::Time { seconds } => write!(
                f,
                "a commit is dated {seconds} seconds from the epoch, which is not a \
                 moment historica can spell"
            ),
            Error::Spawn(error) => write!(
                f,
                "`git` could not be run: {error} — decision 0002 makes it this \
                 crate's one dependency, and it has to be on PATH"
            ),
            Error::GitFailed { status, said } => {
                match status {
                    Some(code) => write!(f, "`git fast-export` exited {code}")?,
                    None => write!(f, "`git fast-export` was stopped")?,
                }
                for line in said {
                    write!(f, "\n  {line}")?;
                }
                Ok(())
            }
            Error::Io { path, error } => write!(f, "{}: {error}", path.display()),
            #[cfg(not(unix))]
            Error::NoLinks { path } => write!(
                f,
                "the repository holds a symbolic link at `{}`, and this platform \
                 has none",
                path.display()
            ),
        }
    }
}

impl error::Error for Error {}

impl From<stream::Error> for Error {
    fn from(error: stream::Error) -> Self {
        Error::Stream(error)
    }
}

impl From<tree::Error> for Error {
    fn from(error: tree::Error) -> Self {
        Error::Replay(error)
    }
}

impl From<identity::Error> for Error {
    fn from(error: identity::Error) -> Self {
        Error::Identity(error)
    }
}

impl From<historica::store::StoreError> for Error {
    fn from(error: historica::store::StoreError) -> Self {
        Error::Store(Box::new(error))
    }
}

impl From<historica::working::WorkingError> for Error {
    fn from(error: historica::working::WorkingError) -> Self {
        Error::Working(Box::new(error))
    }
}
