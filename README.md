# historica-git

Conversion between git repositories and [Historica](https://github.com/diaryx-org/historica)
stores.

Two one-way conversions, each run deliberately and each producing a whole
result: a git repository read into a store, and a store written out as a git
repository. Not a sync, not a remote, not a transport — Historica moves over
whatever already moves a folder, and a git repository moves over whatever
already moves a git repository. What is missing between them is a converter,
and that is all this is.

## Status

Both directions work. A git repository converts into a Historica store, and a
store is written out as a git repository.

```console
$ historica-git import ~/Code/some-repo ~/Code/some-repo-as-historica
read 6 commits, recorded 5 revisions in /Users/adam/Code/some-repo-as-historica/history

bookmarks:
  - main
  - v1.2

what did not cross:
  - 6 commits name a committer other than their author; historica records one
    person, so the author is the revision's and the committer is a
    `git.committer` header beside it
  - one annotated tag did not cross; it is an object with a tagger and a
    message of its own, and historica has nowhere for either — a lightweight
    tag, which is only a pointer, crosses as a bookmark

$ historica-git write ~/Code/some-repo-as-historica ~/Code/some-repo-again
read 5 revisions, wrote 5 commits in /Users/adam/Code/some-repo-again

refs:
  - refs/heads/main
  - refs/tags/v1.2

checked out main

what did not cross:
  - change IDs did not cross; git has nowhere to put one, and decision 0003
    derives one from a commit rather than the other way about
```

`write` rather than `export`, because historica's decision 0042 already gives
`export` to a copy of a store to take away.

A commit is a function of the revision it came from — [decision
0004](docs/decisions/0004-a-commit-is-a-function-of-the-revision.md) — so
converting one store twice, or on two machines, writes the same commits, and
the commit-to-revision correspondence is computed rather than filed. That makes
the round trip checkable with a command you already have:

```console
$ historica-git import repo store && historica-git write store repo-again
$ diff <(git -C repo rev-list --all) <(git -C repo-again rev-list --all)
```

It is the identity for a repository historica can hold entirely, and that now
includes a signed one. A committer distinct from the author and a signature are
facts git keeps in the commit's own bytes and historica has no word for, so
[decision 0005](docs/decisions/0005-the-facts-git-keeps-that-historica-has-no-word-for.md)
carries them across as headers this tool owns — `git.committer`,
`git.signature` — under the door historica's decision 0070 opened. This
repository's own eighteen commits are all signed, all round-trip, and
`git log --show-signature` calls them good in the copy that was written back.

Refs cross too, and the mapping was already written into both designs:
[decision 0006](docs/decisions/0006-a-ref-that-is-only-a-pointer-crosses.md)
makes a branch a bookmark on a *change*, which follows the work through every
rewrite, and a tag a bookmark on a *revision*, which is pinned and cannot move.
So `refs/heads/x` and `refs/tags/x` each go over and come back where they were,
and the written repository has HEAD on a branch and its files in the folder
rather than needing repair before it can be read.

What still does not cross is a message encoding, a submodule, an annotated tag —
an object with a tagger and a message rather than a pointer — and a branch whose
name has a `/` in it, which historica will not hold as a bookmark. Each is
reported rather than dropped quietly.

What is not built is anything that keeps the two in step: no remembered
correspondence, no bookmark named at a converted revision, and no refs crossing
on the way in.

Three decisions everything is written under.

[Decision 0002](docs/decisions/0002-the-bridge-is-a-stream.md) makes the
fast-import stream the whole of historica-git's contact with git: no git library
is linked and no git object is written by this crate. Git itself is the
dependency, and must be on `PATH`.

[Decision 0003](docs/decisions/0003-a-change-is-the-commit-it-came-from.md)
takes a change ID from the git commit's object ID rather than minting one, so
converting a repository twice produces one history rather than two.

[Decision 0001](docs/decisions/0001-what-crosses-the-boundary.md) fixes the
boundary with historica:

- historica-git depends on historica's **published** API and nothing else. A
  fact the API does not expose is a change to historica, not a hole opened
  here.
- The store is written by historica and by nothing else. A conversion that
  hand-writes a revision document would be a second implementation of the
  format, and the format has one.
- A conversion **states what it could not carry**. Each side holds facts the
  other has no place for — a committer distinct from an author, a tag, a merge
  of four parents on one; change IDs, supersession, forgetting on the other —
  and dropping them quietly is the failure that matters, because the person
  holding the result believes it is the thing it came from.
- Whatever crosses can be checked by hand, with `git cat-file` on one side and
  `shasum -a 256` on the other.

Still open, and each its own decision when it is answered: where the
commit-to-revision correspondence is filed, whether a round trip has to be
exact, and where refs and tags go.

The corpus is a stream git wrote, checked in byte-exact, and it checks with the
tool that is already installed:

```console
cd tests/corpus/export && shasum -a 256 -c MANIFEST
```

`tests/corpus/export/make.sh` rebuilds it — run it, read the diff, and commit
that diff deliberately.

## Building

historica is not on crates.io yet, so the dependency is spelled with both a
version and a path, and this repository builds when historica is checked out
beside it:

```console
git clone git@github.com:diaryx-org/historica.git
git clone git@github.com:diaryx-org/historica-git.git
cd historica-git && cargo build
```

## Development

CI is a program rather than a YAML file, as it is in historica. Every job the
workflow runs is one entry in `xtask/src/main.rs`, and `cargo xtask ci` runs all
of them locally, in the same order, against the same commands:

```console
cargo xtask            # what the jobs are
cargo xtask ci         # all of them: fmt, clippy, test, msrv
cargo xtask clippy     # or one
```

### Releasing

`cargo xtask release <patch|minor|major|X.Y.Z>` does the mechanical half — bump
the version, regenerate the changelog's unreleased region into a section under
the new version, commit both, tag — and stops before the push, which is asked
for explicitly each time. See [`docs/CHANGELOG.md`](docs/CHANGELOG.md) for which
half of that file is generated and which is written by hand.

Publishing waits on historica: a crate with a path dependency on an unpublished
crate cannot be published, which is stated rather than discovered in decision
0001.

## Licence

MIT or Apache-2.0, at your option.
