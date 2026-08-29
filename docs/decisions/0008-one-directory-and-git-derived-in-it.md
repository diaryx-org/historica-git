# 0008 — One directory, and git derived in it

0007 deferred this in a sentence, having just built what it needs:

> **Colocation** — the store's folder and the repository's working tree being
> one directory — which is what the second import's rule about the folder was
> written to make possible, and which is its own decision.

The cost of two directories is that a person has to live in one of them. Living
in the repository means `historica record` is somewhere else and every edit is
made where historica cannot see it; living in the folder means `git log`, an
editor's gutter, and every tool that has ever been pointed at a checkout are
looking at a copy. Neither is a conversion problem — both conversions work —
and both end with somebody keeping two directories in their head and
remembering which one they last typed in.

What makes one directory answerable now is that 0007 stopped a second import
touching the folder. The folder is somebody's working copy from the second
conversion onward, and the rule that keeps a conversion out of it is the same
rule that lets git's own directory sit beside it.

## The stance this is written under

**Historica owns the directory, and git is derived in it.**

That is not a new claim. 0004 makes a commit a function of the revision it came
from, which is why the correspondence is computed rather than filed; 0005
carried a signature across as a header, so the derivation survives at the one
place a byte-exact result is hardest to get; and 0007's `commits` file is
already *derived* in historica's 0053 sense — delete it and a whole write
regenerates every line. Colocated, the same thing is true of the projection in
`.git/`: its commits and crossed refs are derived again. Configuration,
remotes, reflogs, hooks, and other local git state are not historica's and are
not promised back.

What colocation adds is that the derivation happens in place, and what this
decision has to settle is what happens when something writes to the derived
side.

## The decision

- **One directory holds `history/`, `.git/`, and the files.** `historica-git
  colocate <directory>` makes the pair from whichever side is already there: a
  repository gets a store built by a whole import replayed in scratch space, a
  store gets a repository built by a whole write, and a directory holding
  neither is refused, because `git init` and `historica init` both exist and
  this is not a third spelling of either. Neither direction touches the files
  already in the directory. On the way in, the folder is adopted as the
  store's working copy at the revision HEAD names, with any staged or unstaged
  difference left as work of its own; on the way out, the repository is made
  around the folder as it stands.

- **`.git` is kept out of history by two private rules, written through
  historica.** `Rule::private(Scope::Under(".git"))` and
  `Rule::private(Scope::Path(".git"))`, added with `Store::add_skipped`. Two,
  because a linked worktree and a submodule spell `.git` as a *file* pointing
  elsewhere, and a rule naming a directory does not cover one. Private rather
  than shared because 0051's axis is exactly this distinction: that there is a
  git repository beside this store is a fact about somebody's machine, not
  about the project, and a copy anybody can fetch should not state it.
  `history/` needs no rule at all — historica's walk already skips its own
  store directory at the root.

- **`history/` is kept out of the repository by git's resolved `info/exclude`,
  not by a `.gitignore`.** A `.gitignore` is a tracked file: it would be recorded into
  the store, cross back out as a commit, and travel to a machine that has no
  repository for it to be about. `info/exclude` is local, untracked, and never
  leaves — the same shape as the private rule on the other side. The path is
  asked of git rather than formed under `.git`, because a linked worktree
  spells `.git` as a file. The tool preserves what is already there and adds
  one idempotent `history/` line of its own. Each side keeps the other out with
  a note that does not travel.

- **A ref that moved on its own is reported, and not followed.** This is the
  whole of the arrangement. Colocated, every commit the repository holds was
  written from a revision, so a ref pointing somewhere `.git/historica/refs`
  does not expect was moved by something that is not this tool — a `git
  commit`, a `git merge`, a `git pull`. The conversion names the ref and the
  commit and does not convert it. **0007's rule is unchanged where the
  repository is a separate directory**: there, git moving a ref is the ordinary
  case and the bookmark follows it. Colocated it is an anomaly, and that
  difference is what colocation is promising.

- **Detection is the default, and an explicit `import` is the override.** The
  work is real and it is in a git object; refusing it forever would make the
  directory hostile. So the report names the command that takes it over, which
  is the conversion that already exists — `historica-git import` run against
  the directory, doing exactly what it does for a separate repository. What
  refuses to follow is the *automatic* half: a conversion run as part of
  something else. A person who types the import is answering the question the
  report asked them: for crossed refs, git wins even where the bookmark also
  moved since the last agreement. The commits are imported, those bookmarks
  are moved to them, and the new agreement is remembered. This is deliberately
  different from 0007's two-directory conflict rule, because naming the same
  directory twice is the explicit choice that resolves it.

- **Git's index is written after every conversion, and the files are never
  touched.** Historica moves the files here, so git's index goes stale on every
  `historica record` and every `historica update`, and a `git status` that has
  not been brought level reports a working tree full of changes that are not
  there. Once a conversion has left HEAD on the commit whose revision the
  folder holds, the index is set to HEAD — `git reset --mixed`, and never
  `--hard`. The distinction is not tidiness: the folder may hold unrecorded
  edits, which are the reason 0007 stopped touching it, and `--hard` would
  destroy them. `--mixed` claims the index, which is git's, and says nothing
  about the files, which are historica's. If an independently moved checked-out
  branch was detected and not followed, HEAD does not name the folder's
  revision and the index is left alone; resetting it to that HEAD would make
  status less truthful rather than more.

- **The record stays in `.git/historica/`, and loses half its job.** 0007 put
  it there because a store may not carry a fact about a repository somewhere
  else, and that is unchanged. What changes is what `refs` is for: "who moved
  this ref" now has one expected answer, so that half becomes the check that
  produces the report above rather than a resolution between two claims. The
  other half — whether a ref is this tool's to delete — is exactly as it was.

- **Nothing carried or shared about the arrangement is in the store.** A store
  colocated on one machine and plain on another is the same store, and still
  converges. The two
  private rules are the only thing written into it, and they are rules about a
  path rather than facts about a repository — which is why they are the shape
  historica already had for this and not a new one.

- **Remote-tracking refs are not this tool's and do not trip anything.** 0006
  crosses `refs/heads/*` and `refs/tags/*`, so a `git fetch` filling
  `refs/remotes/origin/*` is invisible here and stays that way. A `git pull` is
  not: it moves a branch, and that is a ref that moved on its own, reported
  like any other.

## Why detected rather than supported

The alternative is jj's, and it is coherent: import git's refs at the start of
every command, so that anything done with a git tool is absorbed before
anything else happens. It is the right design for jj, whose colocated mode
exists so that a team can adopt it one person at a time while everyone else
keeps using git on the same checkout.

What it costs is that git is an input forever. Every command pays the
reconciliation, `.git/historica/refs` is load-bearing permanently, and 0007's
"both moved is a conflict" rule — written for a case that arises occasionally
between two directories — becomes something a person meets in the course of
ordinary work.

Detection costs one comparison against a file the tool already writes.

And it forbids nothing. `git commit` still runs, the commit still exists, the
person is told it is there and told what to type. What is given up is the
promise that a git tool's *writes* are absorbed silently. What is kept — and it
is the larger half of why anybody wants one directory — is that a git tool's
*reads* are correct: `git log`, `git diff`, `git blame`, and an
editor's gutter all see a repository that is really there and really current.
The index bullet above is what buys that, and it is worth the obligation. Git
commands that check out another tree — `checkout`, `switch`, `restore`, `reset
--hard`, and ordinary `bisect` — write the files historica owns and are outside
the supported arrangement. Their effects are not prevented; the next run sees
them as working-copy edits, not as history to absorb.

## Rejected alternatives

**Importing git's refs at the start of every command.** Above. The right design
for a tool whose colocated mode is an adoption path for a team; the wrong one
for an arrangement whose premise is that historica owns the directory.

**A `pre-commit` hook that refuses.** It would make detection unnecessary by
making the thing detected impossible. Refused three times over: historica's
0053 refuses executable hooks in a store and the reasoning carries here; it
makes the directory hostile to the tools colocation exists to keep working; and
`--no-verify` bypasses it, so it would be a guarantee the tool cannot keep —
which is the objection 0072 made to calling a registry "authorise", arriving
again.

**A `.gitignore` for `history/`.** It travels. See above.

**A shared rather than private rule for `.git`.** It states, in a copy any
stranger can fetch, that the origin keeps a git repository beside its store.
Small, and still somebody's machine rather than the project's.

**`git reset --hard` colocated.** It is what the separate-repository write
already does (`src/export.rs:294`), under a cleanliness check made before the
refs move. Colocated the files are historica's and may hold unrecorded edits at
any moment, so the check would have to be true continuously rather than once,
and the failure is destructive. `--mixed` needs no check.

**A new verb for adopting a commit git made.** `import` already means "read a
repository into a store", and colocated it means that with both of its
arguments being the one directory. A second word for the same conversion under
one condition is a word to explain and a path to test.

**Making colocation the default.** Two directories is right for the person
converting a repository once, which is what this tool was for first. This adds
an arrangement; it does not replace one.

## Consequences

- `import::folder::must_be_free` (`src/import/folder.rs:16`) grows a colocated
  form. A directory holding `.git/`, `history/`, and tracked files is expected
  rather than refused, and a directory holding anything else is refused exactly
  as it is now.
- `colocate` is a third command, and the usage text says what the arrangement
  is rather than only how to spell it.
- `plumbing` gains the index write and the `info/exclude` write. Both are git's
  own plumbing run as a program, which is 0002 unchanged.
- Both `Report`s gain a way to say that a ref moved on its own, which is a
  report and not an error: the conversion does the rest of its work.
- **No `Behavioural-change:` trailer is owed for the separate-repository
  case.** Every rule 0007 wrote for two directories is untouched. A caller who
  never colocates sees no difference, and that is deliberate — this decision
  adds an arrangement beside the existing one rather than altering it.

## Deferred

- **A revision with a contested file has no commit.** Historica's 0032 makes a
  merge state what every contested file is, and a revision holding one cannot
  be written as a git commit that means anything. It is a `write` question
  rather than a colocation question, but colocation sharpens it, because the
  working tree the unwritable revision describes is the directory the person is
  sitting in. What a write does when it reaches one is its own decision.
- **The run around every historica command**, which is what makes colocation
  worth having and what detection will report through. Its own decision, and
  the next one.
- **A rebase, a merge, or anything else that moves several refs at once** is
  detected as several refs that moved on their own, and says so once per ref.
  Correct, and noisier than the one thing that happened.
- **A submodule** puts a second `.git` inside the directory, at a path the two
  rules above do not name. Submodules are already uncarried and reported; this
  adds that they are also unskipped.
- **Windows**, where the index write and `info/exclude` should carry unchanged
  and where nothing here has been run. CI is ubuntu.
