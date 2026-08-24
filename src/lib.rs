//! Conversion between git repositories and Historica stores.
//!
//! Two one-way conversions, each producing a whole result: a git repository
//! read into a store, and a store written out as a git repository. Not a sync,
//! not a remote, not a transport — see `docs/decisions/0001-what-crosses-the-
//! boundary.md`, which also fixes the rule this crate is built under: the store
//! is reached through [`historica`]'s published API and written by nothing but
//! historica itself.
//!
//! Nothing is implemented yet. This crate is the repository, its CI, and the
//! decision that says what it may ask for; the first conversion follows.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
