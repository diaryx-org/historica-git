# 0006 — A ref that is only a pointer crosses

Decision 0001 left "where refs and tags go" open, and both conversions have been
reporting them as uncarried since. That is the last thing between here and using
historica to drive git: a conversion whose branches do not survive is one you
cannot push from.

Historica has bookmarks, and a bookmark names one of three things — a change, a
revision, or a file. Git has refs, and a ref lives in a directory that says what
kind it is. The question is which of those correspond, and the answer turns out
to be written into both designs already.

## The decision

- **A branch is a bookmark on a change; a tag is a bookmark on a revision.**
  Historica's `Name::Change` follows the work through every rewrite, which is
  what a branch does. `Name::Revision` is pinned and cannot move, which is what a
  tag is. So `refs/heads/x` converts to a bookmark on the change and back to
  `refs/heads/x`, and `refs/tags/x` converts to a bookmark on the revision and
  back to `refs/tags/x`. Neither direction invents a grammar, and the round trip
  is exact because the distinction was already carried by the target.

- **An annotated tag does not cross.** A lightweight tag is only a pointer, and a
  pointer is exactly what a bookmark is. An annotated tag is an object with a
  tagger and a message of its own, and historica has nowhere for either. Its
  `from` is deliberately *not* followed into a bookmark: a bookmark there would
  say the tag crossed, when what crossed is where it pointed, and the difference
  is the kind of thing a person only discovers after pushing.

- **A ref that is neither stays where it is.** `refs/remotes/`, `refs/notes/`,
  and whatever else a repository keeps are facts about somewhere else. A
  bookmark claiming otherwise would be this tool deciding what somebody's
  remote-tracking ref meant. They are reported by directory, so a hundred of
  them are one line.

- **Historica decides what a bookmark may be called, and this reports what it
  decided.** A name is refused rather than rewritten, because a name spelled
  some other way is a name nobody can type. The rule is not copied here, where
  it would drift; the refusal is caught and the sentence historica gave for it
  is repeated verbatim.

- **The conversion chooses which branch is checked out, and says which.**
  Nothing in the store says: historica has no HEAD, because the folder *is* the
  tree and `update` is what moves it. So a fresh repository gets `main`, or
  `master`, or failing both the first branch in name order — and a store with no
  branch in it is left with git's own default and a line saying HEAD points at
  nothing. Name order alone would not do, and the reason arrived with nested
  names: `claude/something` sorts above `main`, and a conversion that checked out
  somebody's scratch branch would be arbitrary in a way a person would read as
  broken.

## Why the working tree is populated

`fast-import` moves refs. It does not touch the index or the working tree, and
it cannot make HEAD a *symbolic* ref at all — so a conversion that stopped at
the stream would hand somebody a repository where `git status` reports every
file as deleted. That is a repository they have to repair before they can read
it, which is not what "write the store out as a git repository" should mean.

So `write` finishes with `git symbolic-ref` and `git reset --hard`. Both are git
writing to its own repository, which is decision 0002's arrangement rather than
an exception to it — no git object is written by this crate either way, and the
target was verified empty before anything started, so there is nothing of
anybody's for `--hard` to reach.

## Consequences

- The round trip covers refs now, and the test asserts it: `git for-each-ref` on
  both sides, compared, along with HEAD and a clean `git status`.
- A branch with a `/` in its name does not cross, and that is the largest
  remaining hole in this direction — `feature/x` is an ordinary thing to call a
  branch. It is a question about what historica will hold as a name rather than
  one this tool can answer, which makes it the next thing to take upstream, on
  0001's rule and with a measurement behind it as 0004's was.
- A conversion is still not a sync. Nothing remembers which refs crossed last
  time, so a ref deleted at the origin is not deleted here; it is simply absent
  from a conversion run again into an empty target.

## What is left of the refusal

Historica's name grammar got wider in one direction and narrower in several:
`/` is allowed, and an empty component, `.`, `..`, padding, a control character
and a non-NFC name are all refused where they were not before. Git refuses most
of those in a ref name too, so the sets very nearly agree.

The one that does not is **NFC**. Git does not normalise; historica requires it.
Whether that refusal is ever reached depends on where the conversion runs — git
on macOS precomposes a ref name before storing one, so a decomposed branch
created there arrives already normalised, and elsewhere it does not. The
behaviour is the same either way and is the point: the ref does not become a
bookmark, the conversion finishes, every commit is still in the store, and the
report says which rule the name broke in historica's own words. There is a test
for it built from a stream rather than a repository, because a repository cannot
reach the case on the machine this was written on.

## Deferred
- **An annotated tag**, which needs somewhere for a tagger and a message and is
  therefore the same shape of question decision 0005 answered for a signature —
  but a bigger one, because a tag is an object rather than a header on one.
- **Which branch was checked out**, if it should be a fact the store keeps
  rather than one the conversion picks.
- ~~**Deleting a ref the store no longer names**, which is what a conversion onto
  a repository it made before would have to decide, and which decision 0004's
  "no remembered correspondence" says is not this tool's job yet.~~ Answered by
  [decision 0007](0007-a-conversion-onto-what-it-made-before.md): a write
  deletes a ref it made, still where it left it, that no bookmark is behind —
  and recreates nothing git deleted.
