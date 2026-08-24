# 0001 — What crosses the boundary

historigit converts between git repositories and Historica stores. It is its
own repository rather than a second crate beside historica, and that is the
first thing to write down — not because directory layout deserves a decision
document, but because the reason is a rule about what this tool is allowed to
ask for, and that rule shapes everything after it.

Historica cut 1.0 on two promises: the format, which decision 0047 spells on
line one, and an ordinary semver Rust API. Neither promise is worth anything
if its only serious caller is one that never has to keep it. historigit is
that caller — the first consumer that wants more of a store than a person at a
command line does — so the useful thing it can do for historica, before it
converts a single commit, is to be unable to cheat.

There is precedent for the shape. Decision 0046 put the trust layer outside
historica for the same reason and in almost the same words: the tool gains its
own dependencies and its own grammar, and historica gains neither.

## The decision

- **historigit depends on historica's published API and nothing else.** No
  private path, no `pub(crate)` reached through a workspace, no patched fork.
  If a conversion needs a fact the API does not expose, that is a change to
  historica — with a version number on it and, if it touches the format, a
  decision document behind it — rather than a hole opened here.

- **The store is written by historica and by nothing else.** historigit never
  writes a file under `history/` itself. `check` is historica's invariant to
  hold, and a conversion that hand-writes a revision document is a second
  implementation of the format. The format has one.

- **Two one-way conversions, not a sync.** Git repository to store, store to
  git repository, each run deliberately, each producing a whole result. No
  remembered remote, no tracking ref, no incremental fetch. Decision 0029
  declined to put a transport inside historica and this does not smuggle one
  in from the side: what a conversion produces is a repository, and what moves
  a repository is whatever already moves repositories.

- **A conversion states what it could not carry.** Each side holds facts the
  other has no place for. Git has a committer distinct from an author, tags,
  refs, its own signatures, and merges of more than two parents; historica has
  change IDs that survive rewriting (0001 there), supersession, forgetting
  documents (0014), and resolutions recorded as their own documents (0032).
  Dropping any of that silently is the failure that matters, because the
  person holding the result believes it is the thing it came from. So a
  conversion's output includes a readable account of what did not cross.

- **Whatever crosses can be checked by hand.** The correspondence between a
  git commit and a revision is a readable file, checkable with the tools that
  already check both sides — `git cat-file` on one, `shasum -a 256` on the
  other. This is historica's own non-negotiable rule, applied to the bridge:
  the readable files are the authority.

## What this does not decide

- **Which git library**, or whether the bridge speaks `fast-import` and
  `fast-export` streams and therefore needs none.
- **Where the commit-to-revision correspondence is filed** — in the store, in
  the git repository, or in a file that belongs to neither.
- **What the commands are called.** `export` is taken: decision 0042 gives it
  to historica for a copy of a store to take away, so `historigit export`
  cannot mean "write a git repository" without a collision in the reader's
  head.
- **Whether a round trip has to be exact**, and what it is allowed to lose if
  not.

## Rejected alternatives

**A second crate in historica's workspace.** The dependency weight is not the
argument — workspace members have independent dependency tables and `cargo
build -p historica` never compiles a git library. What is shared is the rest:
one lockfile, one `version = "…"` line that `cargo xtask bump` rewrites and
asserts is unique, one `v*.*.*` tag namespace with no crate name in it, one
`--workspace` CI sweep, and one MSRV floor that would silently become whatever
a git library demands. Historica's 1.0 would then either cover a bridge that
is going to churn, or the release tooling would have to learn which crate a
tag is for. Both are a cost paid to make cheating easy.

**A `git` feature on historica.** Worse than the workspace in the way that
counts. The dependency becomes historica's, and so does every question this
repository exists to answer: what a committer becomes, what a tag becomes,
what happens to a merge with four parents. Those are questions about a
conversion, and answering them inside historica makes them look like questions
about the format.

## Consequences

- historigit cannot be published before historica is. The manifest carries
  both a `version` and a `path` for historica so that the build works today
  and the publish is honest later; a clone of this repository alone does not
  build until one of those two things is true.

- Historica's API gets its first real audit. Anything a conversion cannot do
  without reaching inside is a gap to be filed there — which is the whole
  reason this repository is over here.
