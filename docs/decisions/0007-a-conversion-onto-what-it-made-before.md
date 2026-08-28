# 0007 — A conversion onto what it made before

Decision 0001 fixed both conversions as whole results into an empty target,
and said why: "a conversion only writes where it can be sure it owns
everything it removes." That rule was right and it has a cost, which 0006's
consequences list stated without pricing:

> A conversion is still not a sync. Nothing remembers which refs crossed last
> time, so a ref deleted at the origin is not deleted here; it is simply absent
> from a conversion run again into an empty target.

"Run again into an empty target" is the whole of the cost. A person with a
store and a repository that both keep changing has, until now, been re-reading
every commit and re-writing every revision on each trip, and getting a *new*
repository or a *new* store for their trouble — one they then have to move
their working copy into. Nobody keeps that up. The measurement is this
repository's own history: it was imported, written, and then edited in git,
and the second import went into a third directory.

What makes the second conversion cheap to build is that the first two
decisions already made it almost free to reason about. Decision 0003 makes a
change ID the commit it came from, so a store already says which commits it
holds. Decision 0004 makes a commit a function of its revision, so a
repository already says which revisions it holds — for any revision the tool
has written, `git fast-import --export-marks` handed the object ID back at the
time. Neither side needs to be *told* what the other has. What was missing was
somewhere to keep the one answer that is not derivable, and a rule for the
bookmarks.

## The decision

- **A target that already holds the other side's result is brought up to
  date, not refused.** `import` into a folder that holds `history/` opens the
  store and adds what the repository has gained; `write` into a directory that
  holds `.git` adds what the store has gained. An empty or absent target is
  what it was. Anything else in the way is still refused: a folder with files
  and no store, a directory with files and no repository.

- **The repository keeps the record, in two files under `.git/historica/`.**
  Both are text, one fact per line, checkable by hand, and neither is in the
  store.

  `commits` is `<revision> <object ID>`, one line per revision this tool has
  written as a commit or read from one. It is *derived* in 0053's sense:
  delete it and a whole write regenerates every line, because a commit is a
  function of its revision. What it saves is time — a write names a
  remembered commit by object ID rather than sending it again — and, on the
  way in, it is how a commit this tool wrote is recognised as the revision it
  came from, which 0004 said was the one direction the change ID could not
  answer.

  `refs` is `<ref> <object ID>`, one line per ref the last conversion left
  git and the store agreeing about, and where — with the word `made` after it
  where a write of this tool's is what created or moved the ref. These are the
  facts nothing else holds. When `refs/heads/main` points somewhere the store
  does not expect, the question is *who moved it*, and the only way to answer
  is to remember where it stood when both sides last agreed. And when the
  store stops naming a branch, the question is whether the branch is this
  tool's to delete, which only the marker answers: a branch git had before
  this tool ever wrote here, which an import found agreeing with the store,
  is somebody's, however the store comes to feel about it.

  Both are carried forward and amended, never rewritten from what one run
  saw. A ref held back keeps the line that held it back — otherwise the next
  write would forget why, and undo a deletion somebody meant one run late.

- **They live in the git directory, not the store.** 0004 refused to let a
  store carry a fact about a repository somewhere else, and that refusal
  stands: a store converted on two machines still converges. Historica's 0053
  gives a store directory out only by decision, need first, and the need here
  is the repository's, not the format's. `.git/` is where git's own ecosystem
  keeps a tool's private state — `lfs/`, `info/`, `hooks/` — and a repository
  is the one thing a conversion was already allowed to write into.

  Both conversions keep the record. An import that recorded a commit writes
  the correspondence, so that the next write does not send that revision back
  as a second commit; an import that found a bookmark agreeing with a ref
  writes that too, so that the next write knows the ref is one it may move.
  That an import writes into the repository it reads is deliberate and
  narrow: nothing under `refs/` or `objects/` is touched, only this tool's own
  two files.

- **A second import leaves the folder alone.** The first conversion ends with
  the folder holding the last commit's files, and that stays. A second
  conversion's folder is somebody's working copy, with unrecorded changes in
  it, and historica's 0029 keeps working folders outside a receive; the same
  rule holds here. Each new commit is materialised in a scratch directory,
  `historica record` is asked to read that, and the folder is told nothing.
  `historica update` is what brings it forward, and the report says so.

- **The export is asked for what the store lacks, and a commit standing on a
  held one names it.** Every ref whose commit the store already holds is passed
  to `git fast-export` as an exclusion, with `--reference-excluded-parents`, so
  that the first new commit on a branch states its parent by object ID and is
  written as a difference against it. The replay is handed that parent's tree
  by asking git — `ls-tree` and `cat-file --batch` — which is 0002's "second
  question" clause used for exactly what it was written for. A held commit
  that arrives anyway, because the only ref reaching it has moved on, is
  recognised by what it is and not converted again.

- **A commit is held if the repository's record names it, or if its change is
  in the store.** The second half is 0003 read backwards: the change ID *is*
  the object ID's first twelve bytes, so a commit this tool once imported is
  found in the store without a record at all. The revision it is, among that
  change's revisions, is the one that supersedes nothing — the one recorded
  from the commit, before any amendment.

- **A bookmark follows its ref when git moved it and the store did not, and
  says so when both did.** Where the record says where the two last agreed,
  the rule is symmetrical: on the way in, a ref git moved and a bookmark the
  store has not moved is a bookmark that follows; on the way out, a bookmark
  the store moved and a ref git has not moved is a ref that follows. Both
  having moved is a conflict, reported and not resolved, in whichever
  direction met it first. Where the record says nothing — a repository this
  tool never wrote, or a branch git made since — a bookmark follows a ref only
  if that is a step forward along its own history, and a write never moves a
  ref git already has pointing elsewhere.

- **A write deletes a ref it made that the store no longer names, and
  recreates nothing git deleted.** Both need the record. A ref the last write
  left, still where it was left, with no bookmark behind it, is deleted — that
  is 0006's deferred question, answered. A ref the last write left that git
  has since deleted, with a bookmark still behind it, is held back rather than
  brought back: recreating it would undo a deletion somebody meant.

- **An import reports a branch git deleted and cannot delete the bookmark.**
  Historica's published API sets a bookmark and does not remove one. Under
  0001 that is a change to historica rather than a file to delete from here,
  and until it exists the conversion says what happened and leaves the
  bookmark where the branch was.

- **`record`'s own bookmark-advancing is undone as it happens.** Historica's
  0011 has `record` carry forward every bookmark that named a parent's change,
  which is right for a person recording onto a branch and wrong for a
  conversion, where git says where its branches are. So after each commit is
  recorded, whatever `record` advanced is put back, and the one thing an
  import does to a bookmark is done once, at the end, from git's refs. Without
  this, importing a `topic` branch ahead of `main` would drag `main` up
  `topic` in the store, and the conflict rule above would then read that as
  the store having moved it.

- **A second write brings the working tree up only where nothing of anybody's
  is in the way.** The repository's HEAD is git's; the conversion does not
  choose a branch for it. Where HEAD's branch moved and the index and working
  tree were clean *before the refs moved* — asked then, because afterwards an
  untouched index reads as a tree full of deletions against the new commit —
  `git reset --hard` brings the files forward, and that is the only case in
  which a second write touches the working tree. Otherwise it says what it
  left and why.

## Why this is still not a sync

0001 refused a sync, and 0029 in historica said what the word means: a
remembered peer, bidirectional mutation, transport, incremental negotiation,
and a policy for unrecorded working changes. This decision has none of those.
There is no remembered peer — the record is *in* the repository, about that
repository, and a store carries nothing. Each command still mutates one side.
There is no transport; the repository moves by whatever moves repositories.
The negotiation is git's own `^commit`, which is not a protocol. And the
policy for unrecorded changes is that there is none: they are not looked at.

What it is, in 0029's terms, is the content-aware operation that plain copying
stops being sufficient for — and 0029 built one of those too, and called it
`receive` rather than `sync` for the same reason.

## Rejected alternatives

**Keeping marks files across runs.** `--import-marks`/`--export-marks` is
git's own mechanism for an incremental `fast-import`, and 0002 deferred
exactly this. It is refused because marks are numbered per stream, so a
remembered mark file would have to be joined against this tool's own numbering
anyway; and because it holds one direction only. A `<revision> <object ID>`
file is the join already done, readable without git, and answers both
questions. The marks file is still how the object IDs are *learned*, and it is
deleted once read.

**Deriving "held" on the way out from the change ID alone.** It is tempting
to say a written commit is recognisable by construction, since 0004 makes it
a function of its revision — but computing that function *is* running
`fast-import`, and the object ID cannot be had cheaper than by sending. So a
write that remembered nothing would re-send everything, and git would dedupe
it, which is correct and is exactly the cost this decision exists to stop
paying.

**Exporting everything and skipping what is held.** Simpler, and it works —
the skip is in place regardless, for the held commit that arrives anyway. It
is refused as the *only* mechanism because it makes a second import cost the
whole repository's bytes through a pipe on every run, which is the shape of
cost that stops a person running it.

**Reading the boundary tree from the store rather than from git.** The
store's tree at the imported revision is *almost* git's tree at the commit,
and 0004 lists the ways it is not: a link held as an identity, a contested
file. A replay seeded from the store would apply git's difference to
historica's approximation of the base and materialise something that is
neither. Git has the tree exactly, and asking it is what 0002 allowed.

**Remote-tracking bookmarks.** A branch that moved in git and not in the
store could be shown as a divergence between `main` and `main@origin`, as jj
does, rather than followed or refused. 0029 deferred remote-tracking bookmarks
and this decision does not reach for them, because the names would have to
live in `names/`, where receive would treat two machines' `main@origin` as a
conflict of the whole receive. The rule here needs no names, only the record.

**A store directory for the record.** 0053's reservation, with the `derived`
class for `commits` and `local-only` for `refs`. Refused because it would be
asking historica for a directory in order to file a fact about a repository
somewhere else, which is what 0004 promised the store would never hold, and
because a store converted on two machines would then differ in a file that
travels with `cp -r`.

## Consequences

- `plumbing` is a new module: the questions asked of git besides the stream,
  each git's own plumbing run as a program. `remembered` is another: the two
  files, their grammar, and nothing else.
- `import::Report` gains `held`, `moved`, and `onto`; `export::Report` gains
  `reused`, `deleted`, and `onto`, and `branch` is now only the branch a
  working tree was actually brought up to. All are public, so the implementing
  commit carries `Behavioural-change:` trailers — as does the target rule,
  since a caller who relied on a non-empty target being refused will find a
  store or a repository there accepted.
- `Trees` can be seeded with a tree for a commit the stream names, and can be
  asked what tree a commit starts from. The second fixes a fault the first
  conversion had: a directory rename was expanded against whatever tree was
  on disk last, which is the previous commit in *stream* order and another
  branch's whenever the stream switches. It is now expanded against the
  commit's own parent.
- `from_stream` is unchanged in contract: a stream names nothing this tool
  could ask git about, so a commit it only names is still refused, and the
  corpus tests stand as they were.
- The round trip that 0004 makes a test now includes a second lap: a commit
  made in the written repository comes back standing on the revision its
  parent was written from, and a write after that has nothing to send.

## Deferred

- **Removing a bookmark through historica's API**, which is the one thing an
  import cannot carry across. Filed upstream under 0001's rule.
- **Excluding held commits that are not ref tips.** The exclusion set is the
  refs whose commits are held, so a new branch grown from an old point re-reads
  the commits between; they are recognised and skipped, and the cost is git's
  bytes rather than historica's writes. Walking each unheld tip back to the
  nearest held commit would close that, at the cost of one `rev-list` per tip,
  when somebody measures a repository where it matters.
- **A commit that changed nothing** maps to its parent's revision and is kept
  out of the record, since a record naming two commits for one revision would
  answer the reverse question two ways. It is re-read on every import and
  folded into its parent again, which is cheap and slightly untidy; and a new
  branch grown from one names a commit neither the record nor the store can
  place, so the import asks git for its parents and walks the first-parent
  line until it reaches one it can. That works and is a second question git
  did not need to be asked; recording the empty commit against its parent's
  revision in a form that does not disturb the reverse lookup would remove it.
- **A branch git deleted is reported on every import** until the bookmark is
  removed, since the record keeps the line that would otherwise let a write
  recreate the branch. Noisy rather than wrong, and gone the day historica's
  API can remove a bookmark.
- **Colocation** — the store's folder and the repository's working tree being
  one directory — which is what the second import's rule about the folder was
  written to make possible, and which is its own decision.
- **Running both directions as one command**, and running one around every
  historica command, which is what a person means by seamless and which this
  decision only makes buildable.
