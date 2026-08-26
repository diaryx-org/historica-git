//! Conversion between git repositories and Historica stores.
//!
//! Two one-way conversions, each producing a whole result: a git repository
//! read into a store, and a store written out as a git repository. Not a sync,
//! not a remote, not a transport — see `docs/decisions/0001-what-crosses-the-
//! boundary.md`, which also fixes the rule this crate is built under: the store
//! is reached through [`historica`]'s published API and written by nothing but
//! historica itself.
//!
//! Git is reached as a program rather than as a library: `git fast-export`
//! writes a stream and `git fast-import` reads one, and [`stream`] is that
//! stream both ways. Decision 0002 says why, and what would reverse it.
//!
//! Both conversions are implemented. [`import`] reads a repository into a
//! store; [`export`] writes a store out as a repository, under decision 0004's
//! rule that a commit is a function of the revision it came from and so needs
//! no remembered correspondence.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod export;
pub mod identity;
pub mod import;
pub mod stream;
pub mod tree;
