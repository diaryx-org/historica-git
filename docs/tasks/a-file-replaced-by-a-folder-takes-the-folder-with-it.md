---
title: A file replaced by a folder takes the folder with it
part_of: '[Tasks](/historica-git/docs/tasks/tasks.md)'
created: 2026-11-07
updated: 2026-11-07
status: done
description: A commit that turns the file `p` into a folder `p/` imports without what it put in the folder, and says nothing, because the stream states `M p/q` before `D p` and the replay takes them in that order; moving the file aside instead fails the import
---

# A file replaced by a folder takes the folder with it

**Status.** Done, by `fix(import): keep what a commit wrote beneath a file it
replaced`. A `D`, `R` or `C` that names a file now takes that file and not the
paths beneath it, in `Tree::named`. Changes stay in the order the stream states
them, which the experiment under *What was tried* gave up: it would have read
`M a` then `R a b` the wrong way round. A store imported before this keeps the
commits it lost; a fresh import is the remedy.

A commit that replaces a file with a folder of the same name loses the folder's
contents on `import`. The import exits 0 and reports nothing under what did not
cross. The README's promise is that what does not cross is reported, and that
the round trip is the identity for a repository historica can hold entirely.
Historica can hold this one.

## Repro

```console
$ git init -q -b main repo && cd repo
$ echo a > p && git add -A && git commit -qm file
$ rm p && mkdir p && echo b > p/q && git add -A && git commit -qm dir
$ cd .. && historica-git import repo store
read 2 commits, recorded 2 revisions in store/history

bookmarks:
  - main
$ historica-git write store again >/dev/null
$ git -C repo ls-tree -r --name-only main
p/q
$ git -C again ls-tree -r --name-only main
$
```

The written commit has the empty tree, and `store/` holds no `p`. Every file
under the new folder goes, at any depth; a file elsewhere in the commit
survives. A later commit that writes `p/q` again brings it back from there on,
so the head can look right over a wrong commit, and every commit after the
loss has a different object ID from the one it came from.

Moving the file aside rather than deleting it, from the same first commit,
fails instead:

```console
$ git mv p old && mkdir p && echo b > p/q && git add -A && git commit -qm dir
$ historica-git import repo store
historica-git: store/old: File exists (os error 17)
```

The other direction, a folder replaced by a file, is sound.

## Why

`git fast-export` sorts a commit's changes deepest path first, so that a folder
that becomes a file is emptied before the file is written. For the opposite
change that order is the wrong way round:

```text
M 100644 6178079… p/q
D p
```

`Tree::commit` in `src/tree.rs` applies the changes in the order stated, and
`Change::Delete` removes everything `Tree::beneath` the path, so `D p` takes the
`p/q` written one line earlier. With `-M`, which `import` passes, the second
case is `M p/q` then `R p old`: the rename carries `p/q` along too, and `old`
is then both a file and a folder.

This much is git's. `git fast-export | git fast-import` loses `p/q` the same way
(measured on git 2.56.0: the new repository's head has an empty tree and another
object ID), so the stream is not a faithful description of the commit if read in
order. That is a reason for `import` to read it differently, not a reason to
keep the loss: `import` is the one that promises a whole result.

## What was tried

Applying each commit's `Modify` changes after its others, in `Tree::commit`
alone, took six repositories to identical object IDs through `import` and
`write`: the two repros above, a nested folder beside an unrelated file, a later
commit over the loss, a folder replaced by a file, and a file swapped to a folder
and back with a move after it. The existing tests pass with it. That was an
experiment and was not kept. It does not settle a commit that renames `a` and
writes a new `a` in the same breath, which no test here covers and which the
fix should check, since a rename there must still read the old `a`.

## Done when

A test in `tests/again.rs` holds each repro above to the identity the README
states, `git rev-list --all` agreeing on both sides, and the same for a commit
that renames a file and writes a new one at its old path.
