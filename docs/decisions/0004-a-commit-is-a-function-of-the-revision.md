# 0004 — A commit is a function of the revision

Decision 0001 built the reading half and left three things open, and a store
written out as a git repository walks into all three at once: whether a round
trip has to be exact, where the commit-to-revision correspondence is filed, and
what the command is called.

They look like three questions. They are one, and the reason is that the
correspondence is only worth filing if it cannot be computed. A git commit's
object ID is the digest of its own bytes — tree, parents, author line,
committer line, message — so if every one of those is fixed by the revision,
the object ID is fixed by the revision too, and there is nothing to remember.
If any of them is chosen at conversion time, there is, and a store starts
carrying a fact about a repository somewhere else.

## The decision

- **Nothing about a commit is chosen at conversion time.** The tree is the
  revision's tree, the parents are its parents converted, the author line is
  its `author` and `when`, and the message is its message. **The committer is
  the author**, restated rather than read from the clock or the environment,
  because a committer that is the converting machine would make the object ID
  a fact about who ran the conversion.

- **So the correspondence is derived, and filed nowhere.** A conversion run
  twice writes the same commits; a conversion run on another machine writes the
  same commits; and `git fast-import --export-marks` hands back the object IDs
  whenever anybody actually wants them. Under decision 0053's classes that
  makes the map `derived` — deletable without loss, costing time and never
  information — so it needs no reserved directory, and this repository asks
  historica for nothing to store it in.

- **A round trip is exact where historica holds the whole commit, and says so
  where it does not.** This is the honest form of 0001's deferred question. A
  commit whose committer was its author, that carried no signature, no
  `encoding`, and no submodule, converts to a revision and back to *the same
  object ID*. A commit that carried any of those does not, because import
  already had nowhere to put them and said as much. The set that does not
  round-trip is exactly the set import reported as uncarried, which is not a
  coincidence but the same fact read from the other end.

- **The direction is spelled `write`.** 0001 ruled out `export` — historica's
  decision 0042 gives that word to a copy of a store to take away, and one
  binary called `historica-git` cannot use it for something else without the
  collision landing in the reader's head. `write` is the verb the README
  already uses for this direction in prose: a store *written out* as a git
  repository.

- **What a conversion cannot write, it refuses rather than approximates.** A
  file whose content two branches contested has no content at that revision —
  historica's decision 0008 declines to pick a winner — and git has no way to
  say so. Writing either side would be inventing the answer historica refused
  to invent.

## Why the committer is not the converting machine

It is the only line with a free choice in it, and every other property here
follows from closing it.

Git itself would fill a committer from `user.name`, `user.email`, and the
clock. Any of the three moves between machines, and all three move between
people, so a conversion that took them would produce a different repository for
every person who ran it over the same store. The commits would hold the same
work, hash differently, and push as a rewrite of each other's history — which
is the failure that makes a bridge unusable for the thing a bridge is for.

Restating the author costs the ability to record who ran the conversion, which
is a fact nobody has asked for and which git's reflog records anyway.

## What does not cross, and why each one is not a defect

Written down here rather than discovered by a person holding a repository,
which is 0001's rule about stating what could not be carried:

- **Supersession.** Historica's rewriting is a first-class fact and git has no
  equivalent — a superseded revision is a commit git would simply not have. So
  a conversion writes the current revision of each change and leaves the
  superseded ones out, and says how many it left.
- **A contested file.** Above: refused, named, and counted.
- **Forgotten payloads.** Historica's decision 0014 lets a file's bytes be
  destroyed while its history stands. Git has no absent blob, so a revision
  reaching one cannot be written whole.
- **A link historica holds as an identity.** Decision 0040 records a symlink
  that resolves inside the history as the *file* rather than as the string, so
  it keeps pointing at that file across a rename — which is precisely the case
  git gets wrong, leaving the link dangling. Writing that back out has to state
  a path, and the path is the one the file is at now. So a repository holding a
  symlink that was already broken when it was imported converts to one where it
  works, and the commit is not the commit it came from. This is the one
  divergence here that is historica being *right*, and it is still a
  divergence, so it is counted and said.
- **Change IDs.** They go nowhere. Decision 0003 derives one *from* a commit,
  and the inverse does not exist: 96 bits of a 160- or 256-bit object ID is not
  enough to name the commit it came from, which is the whole reason the marks
  file is how object IDs are learned.

## Consequences

- Import then write, over a repository historica can hold entirely, is the
  identity. That is a test rather than an aspiration: `git rev-list --all` on
  both sides, compared. It is also 0001's rule about checking by hand, met with
  a command a person already has.
- The gap between what round-trips and what does not becomes measurable rather
  than argued, and the measurement is the specification for whatever closes it.
  Measured on the first repository it was pointed at: three commits carrying a
  symlink, an executable, a file of bytes, a path with a space in it and a
  rename, of which the two before the rename reproduced their object IDs
  exactly and the third differed in one blob — the link, for the reason above.
  Measured on this repository: eighteen commits, every one signed, none of which
  round-tripped. That number was the argument for historica's 0070, and decision
  0005 is what closed it.
- Two people converting one store and pushing to one remote push the same
  commits, so the second push is a no-op rather than a rewrite.

## Deferred

- ~~**Carrying the committer, the encoding, and the signature across.**~~
  Answered by [decision 0005](0005-the-facts-git-keeps-that-historica-has-no-word-for.md),
  once historica's 0070 built the field this was waiting on — which is 0001's
  rule working exactly as intended. The committer and the signature cross; the
  encoding is a different kind of gap and is argued there.
- **Submodules.** Git's `160000` entry names a commit of another repository and
  historica has no file that is one. Neither direction carries it.
- **Where bookmarks become refs**, beyond the obvious mapping, and what becomes
  of a private bookmark under historica's decision 0062.
- **Whether the stream is kept.** Decision 0002 says a conversion can be asked
  for the stream it sent; this does not yet decide the flag that asks.

## Since

"Filed nowhere" above is now "filed nowhere *for correctness*."
[Decision 0007](0007-a-conversion-onto-what-it-made-before.md) keeps a
`<revision> <object ID>` file in the repository — under `.git/historica/`,
never in the store — so that a second write can name a commit git already has
rather than send it again. The argument here is untouched: the file is
derived, deleting it costs a whole write and nothing else, and the store still
carries no fact about any repository.
