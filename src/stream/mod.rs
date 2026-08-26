//! The fast-import stream: what git says a repository is when asked in text.
//!
//! Decision 0002 makes this the whole of historica-git's contact with git.
//! Reading a repository is `git fast-export` writing this stream; writing one
//! is `git fast-import` reading it. There is no git library here and no git
//! object is ever written by this crate.
//!
//! The model below is the stream's own vocabulary rather than a translation of
//! it. That is deliberate: a parser that renamed things as it went would make
//! every question about fidelity — what a committer becomes, what a tag becomes
//! — a question about this module, and decision 0001 puts those questions in
//! the conversion instead. What arrives here is what git said, spelled the way
//! git spelled it.
//!
//! Paths are bytes. Git's are, and decision 0033 in historica — which asks for
//! one spelling of a path, normalised — is a rule about what a store may hold,
//! not about what a stream may say. Normalising here would lose the ability to
//! report the path git actually had.

mod quote;
mod read;
mod write;

pub use quote::quote;
pub use read::{Error, Reader};
pub use write::Writer;

/// A mark: the stream's own name for an object it has just described, so that
/// later commands can refer to it before git has assigned it an object ID.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Mark(pub u64);

/// How a command names content it did not just write inline.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DataRef {
    /// `:7` — a mark set earlier in this stream.
    Mark(Mark),
    /// A hex object ID, for content the stream did not carry. `git fast-export
    /// --no-data` produces these, as does `--reference-excluded-parents`.
    Oid(String),
}

/// The file modes the stream can state, which are git's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// `100644` — an ordinary file.
    File,
    /// `100755` — a file with the executable bit, which historica records too
    /// (its decision 0034).
    Executable,
    /// `120000` — a symbolic link, whose content is the target path.
    Symlink,
    /// `160000` — a commit of another repository: a submodule.
    Gitlink,
    /// `040000` — a subdirectory, which only `--full-tree` streams state.
    Directory,
}

impl Mode {
    /// The mode as the stream spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::File => "100644",
            Mode::Executable => "100755",
            Mode::Symlink => "120000",
            Mode::Gitlink => "160000",
            Mode::Directory => "040000",
        }
    }
}

/// Who did something, and when they say they did it.
///
/// The time is the stream's `raw` format — seconds since the epoch and the
/// offset the person was at — kept as the two numbers rather than resolved into
/// one instant, because the offset is a fact about the commit that resolving
/// would throw away.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Person {
    /// The name, which git allows to be empty.
    pub name: Vec<u8>,
    /// The address between the angle brackets.
    pub email: Vec<u8>,
    /// Seconds since the Unix epoch.
    pub seconds: i64,
    /// Minutes east of UTC: `-0700` is `-420`.
    pub offset_minutes: i32,
}

/// One thing a commit did to the tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Change {
    /// `M <mode> <dataref> <path>` — a file put at a path, whether or not it
    /// was there before.
    Modify {
        /// The mode the file takes.
        mode: Mode,
        /// The content, either named or carried inline by `inline`.
        content: Content,
        /// Where it goes.
        path: Vec<u8>,
    },
    /// `D <path>` — a file or directory removed.
    Delete {
        /// What is removed.
        path: Vec<u8>,
    },
    /// `R <source> <destination>` — a rename, which `git fast-export -M`
    /// produces and which historica records as a rename too.
    Rename {
        /// Where it was.
        source: Vec<u8>,
        /// Where it now is.
        destination: Vec<u8>,
    },
    /// `C <source> <destination>` — a copy, from `-C`.
    Copy {
        /// What was copied.
        source: Vec<u8>,
        /// The new path, the source keeping its own.
        destination: Vec<u8>,
    },
    /// `deleteall` — the tree is emptied, and what follows states it whole.
    DeleteAll,
    /// `N <dataref> <commit>` — a note attached to a commit.
    Note {
        /// The note's text.
        content: Content,
        /// What it is a note about.
        commit: DataRef,
    },
}

/// Content a command names, or carries itself.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Content {
    /// Named by mark or object ID.
    Named(DataRef),
    /// `inline`, followed by the bytes themselves.
    Inline(Vec<u8>),
}

/// `blob` — content, filed under a mark for later commands to point at.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Blob {
    /// The mark this blob is filed under, if it was given one.
    pub mark: Option<Mark>,
    /// `original-oid`, from `--show-original-ids`.
    pub original_oid: Option<String>,
    /// The bytes.
    pub data: Vec<u8>,
}

/// `commit` — one revision, and what it did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Commit {
    /// The ref this commit is committed to, such as `refs/heads/main`.
    pub reference: Vec<u8>,
    /// The mark it is filed under, if any.
    pub mark: Option<Mark>,
    /// `original-oid`, from `--show-original-ids`.
    pub original_oid: Option<String>,
    /// Who wrote it. Absent when git had only a committer to state.
    pub author: Option<Person>,
    /// Who committed it. Git always states this one.
    pub committer: Person,
    /// `encoding`, when the message is not UTF-8.
    pub encoding: Option<Vec<u8>>,
    /// The message, exactly as it was, without a trailing newline added or
    /// removed.
    pub message: Vec<u8>,
    /// The first parent, absent on a root commit.
    pub from: Option<DataRef>,
    /// Every further parent. Git allows more than one, so this is a list and
    /// not a second `Option` — an octopus merge is a thing a conversion has to
    /// answer for rather than a thing this refuses to read.
    pub merges: Vec<DataRef>,
    /// What the commit did, in the order it was stated.
    pub changes: Vec<Change>,
}

/// `tag` — an annotated tag.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Tag {
    /// The tag's name, without `refs/tags/`.
    pub name: Vec<u8>,
    /// The mark, from `--mark-tags`.
    pub mark: Option<Mark>,
    /// What it tags.
    pub from: DataRef,
    /// `original-oid`, from `--show-original-ids`. It comes after `from` here
    /// rather than after `mark` as it does on a blob or a commit, because that
    /// is where git writes it.
    pub original_oid: Option<String>,
    /// Who made it. A tag written without one says so.
    pub tagger: Option<Person>,
    /// The tag message.
    pub message: Vec<u8>,
}

/// `reset` — a ref moved, with no commit of its own. Lightweight tags and
/// branch creation both arrive this way.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reset {
    /// The ref being moved.
    pub reference: Vec<u8>,
    /// Where it is moved to. Absent when the reset only deletes.
    pub from: Option<DataRef>,
}

/// One command from the stream.
///
/// The variants git's own frontends use to talk back to git — `checkpoint`,
/// `get-mark`, `cat-blob`, `ls`, `alias` — are not here. They cannot appear in
/// `git fast-export` output, and reading them would mean answering them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Command {
    /// Content.
    Blob(Blob),
    /// A revision.
    Commit(Commit),
    /// An annotated tag.
    Tag(Tag),
    /// A ref moved.
    Reset(Reset),
    /// `progress` — a line git echoes back untouched, for a person watching.
    Progress(Vec<u8>),
    /// `feature` — something the stream requires of whoever reads it.
    Feature(Vec<u8>),
    /// `option` — something the stream asks of whoever reads it.
    Option(Vec<u8>),
    /// `done` — the stream says it ended, rather than merely stopping.
    Done,
}
