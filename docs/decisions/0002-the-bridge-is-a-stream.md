# 0002 — The bridge is a stream

Decision 0001 left open how historigit reaches a git repository at all. Two
answers: link a git library and read objects, or run git and speak the
interchange format it already has.

## The decision

- **Both directions are the fast-import stream.** Reading a repository is
  `git fast-export` writing that stream; writing one is `git fast-import`
  reading it. historigit links no git library and writes no git object.

- **git is a dependency, and a declared one.** It must be on `PATH`, at a
  version holding the flags a conversion uses. The floor is checked and
  reported, not assumed — a missing flag must fail saying which git is needed,
  not by producing a conversion that quietly carried less.

- **The stream is an artifact, not a pipe's interior.** A conversion can be
  asked to keep the stream it sent or received. That is decision 0001's rule
  about checking by hand, met: what crossed is a file a person can read, diff,
  and replay with git alone.

## Why this is less code, not more

Writing a git repository means writing trees: entry ordering, mode encoding,
nesting, object writing, refs. `git fast-import` does all of it from `blob`,
`data`, `commit`, and `M <mode> :<mark> <path>`. Reading one the other way,
`git fast-export -M` states each commit as a **change list against its parent**
with renames already detected — which is the shape historica records in, its
decision 0007. A library hands over trees instead, and getting back to that
list means diffing them and re-detecting the renames git had already found.

What is left is a strict line-oriented parser over counted binary blocks, held
to a corpus. That is the thing this house already knows how to build.

## Why a library is the larger maintenance bill

`gix` is pre-1.0 across a large tree of crates, so its breaking releases and
its MSRV would both become this repository's calendar. `git2` is libgit2, a C
library, which historica's decision 0046 already declined for the trust layer
and for the same reason.

Against that, the stream format is git's own compatibility interface. It
cannot break without breaking every importer ever written against it, which is
a stability promise no 0.x crate can make.

## What it costs

- git on `PATH`. A conversion is not a pure function of this crate.
- Subprocess plumbing, where both pipes must be drained or a large conversion
  deadlocks on a full buffer.
- End-to-end tests need git installed. The reader's own tests do not: they run
  against a checked-in stream.
- Some facts need a second question to git — `cat-file`, `rev-parse` — because
  the stream does not carry them. That is still no library.

## What would reverse this

Written down so the question can be reopened on evidence rather than on taste,
as `historica/docs/loro.md` does for its own choice:

- A fact a conversion needs that the stream cannot express **and** git's
  plumbing cannot supply.
- The subprocess boundary becoming the limit on a conversion of a real
  repository, measured rather than assumed.
- Git gating the stream behind a flag, or deprecating it.

## Deferred

- Which `--signed-commits` mode a conversion asks for, and what a stripped
  signature is recorded as. (The default matters: git refuses a signed **tag**
  outright unless told what to do with it, which is a failure a person will
  meet on a real repository.)
- Whether marks files are kept, which is what would make a conversion
  incremental.
- What `encoding` on a commit becomes, given historica's documents are UTF-8.
