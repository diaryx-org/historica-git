# Changelog

What has changed in historica-git, release by release, for someone deciding
whether to move to a newer one.

Two halves, written two different ways.

The bulleted groups below — **Added**, **Fixed**, **Changed**, and a
**Behavioural changes** section under them — are **generated** from the commit
log by `cargo xtask changelog --write`, which reads `.config/cliff.toml`.
Anything inside a `git-cliff:begin` / `git-cliff:end` pair is rewritten on every
run, so an edit made there is an edit thrown away.

Everything else is handwritten and stays: this prose, and any intro a release
needs under its own heading, below the end marker where regeneration cannot
reach it.

**Behavioural changes** are collected from `Behavioural-change:` trailers on the
commits themselves, not from their subjects — because "would a reader who
upgrades without editing a line of their own code observe a difference" is a
judgment about the change that no subject can carry. Write one trailer per
observable difference, as prose someone can act on.

historica-git has not been released, and cannot be until historica is: decision
0001 explains why the dependency is spelled with both a version and a path.

## Unreleased

<!-- git-cliff:begin — generated; edits here are overwritten -->

### Breaking

- **carried** — carry the committer and the signature across as headers ([`5090b85`](https://github.com/diaryx-org/historica-git/commit/5090b8510e3df210f3fc536015f3aacb643445cb))

### Added

- the repository, its CI, and the boundary a conversion works under ([`b35d2af`](https://github.com/diaryx-org/historica-git/commit/b35d2af90e4486a9053b8010c36b597425f20219))
- **stream** — read what git fast-export writes ([`5261ef1`](https://github.com/diaryx-org/historica-git/commit/5261ef12da01564e0c3bd0a220169f87e863d660))
- **tree** — replay a stream into the tree at each commit ([`a3f8dd0`](https://github.com/diaryx-org/historica-git/commit/a3f8dd05d1859fe77933a3c66c6e1d174f0e86c1))
- **identity** — derive a change from the commit's object ID ([`0be7985`](https://github.com/diaryx-org/historica-git/commit/0be798563d1f7eb762249b683cba90f81e3bdf56))
- **import** — convert a git repository into a store ([`5ce6710`](https://github.com/diaryx-org/historica-git/commit/5ce671063b245957c2badb327bb0964da4d79dbc))
- **export** — write a store out as a git repository ([`784e768`](https://github.com/diaryx-org/historica-git/commit/784e7687815106f391b74f4bb54f7d517d5b9357))

### Fixed

- **stream** — read the object ID a tag carries ([`ff8e64c`](https://github.com/diaryx-org/historica-git/commit/ff8e64caa69107692bad6f57d496b53a2b20b961))
- **import** — state no kind, which is what git has to say about one ([`28eb29d`](https://github.com/diaryx-org/historica-git/commit/28eb29d61e4c27baf20704eb89fa94392d27c6b0))

### Behavioural changes

- A revision imported from git now carries `git.committer` and
  `git.signature` headers where the commit had them, and those headers are in
  the canonical bytes. So the same repository imported by this version and by
  the last one produces different revision IDs, and two such stores will not
  fold together — `receive` will union them as unrelated history. Re-import
  rather than mix them.

- `import` asks git for `--signed-commits=verbatim` rather
  than `--signed-commits=warn-strip`. A signed commit's signature now reaches
  the store instead of a warning reaching stderr, and the report no longer says
  that signatures were stripped, because they no longer are.

- `stream::Commit` gained a `signature` field and
  `stream::Signature` is new, so code constructing a `Commit` literal no longer
  compiles until it states one. `None` is what a stream carrying no signature
  reads as.

<!-- git-cliff:end -->
