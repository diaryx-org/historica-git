# Changelog

What has changed in historigit, release by release, for someone deciding
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

historigit has not been released, and cannot be until historica is: decision
0001 explains why the dependency is spelled with both a version and a path.

## Unreleased

<!-- git-cliff:begin — generated; edits here are overwritten -->

### Added

- the repository, its CI, and the boundary a conversion works under ([`b35d2af`](https://github.com/diaryx-org/historigit/commit/b35d2af90e4486a9053b8010c36b597425f20219))
- **stream** — read what git fast-export writes ([`5261ef1`](https://github.com/diaryx-org/historigit/commit/5261ef12da01564e0c3bd0a220169f87e863d660))
- **tree** — replay a stream into the tree at each commit ([`a3f8dd0`](https://github.com/diaryx-org/historigit/commit/a3f8dd05d1859fe77933a3c66c6e1d174f0e86c1))
- **identity** — derive a change from the commit's object ID ([`0be7985`](https://github.com/diaryx-org/historigit/commit/0be798563d1f7eb762249b683cba90f81e3bdf56))
- **import** — convert a git repository into a store ([`5ce6710`](https://github.com/diaryx-org/historigit/commit/5ce671063b245957c2badb327bb0964da4d79dbc))

### Fixed

- **stream** — read the object ID a tag carries ([`ff8e64c`](https://github.com/diaryx-org/historigit/commit/ff8e64caa69107692bad6f57d496b53a2b20b961))

<!-- git-cliff:end -->
