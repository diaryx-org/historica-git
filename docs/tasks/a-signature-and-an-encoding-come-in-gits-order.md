---
title: A signed commit's encoding is read out of git's order
part_of: '[Tasks](/historica-git/docs/tasks/tasks.md)'
created: 2026-11-08
status: open
description: The stream reader wants `encoding` before `gpgsig` and the writer writes them that way, but git puts `gpgsig` first, so a signed commit that states an encoding is refused when it is read, and a stream that is written with both is refused by `git fast-import`
---

# A signed commit's encoding is read out of git's order

git's fast-import grammar gives a commit's headers in this order:

```text
'committer' (SP <name>)? SP LT <email> GT SP <when> LF
('gpgsig' SP <algo> SP <format> LF data)?
('encoding' SP <encoding> LF)?
data
```

`git fast-export` writes them in that order, and `git fast-import` reads them
in that order and no other. `stream::Reader::commit` (`src/stream/read.rs`,
line 159) reads `encoding` first and then `gpgsig`. Its comment says that is
"where git writes it", but git does not write it there.
`stream::Writer::commit` (`src/stream/write.rs`, line 88) uses the reader's
order. The reader and the writer agree with each other, so
`a_signed_commit_survives_being_read_and_written` passes, but its fixture has no
`encoding`, so it never tests the order.

## Repro

A signed commit whose message is not UTF-8, exported without re-encoding:

```console
$ git init -q r && cd r
$ ssh-keygen -q -t ed25519 -N '' -f ../key
$ git config gpg.format ssh && git config user.signingkey ../key
$ git config i18n.commitEncoding ISO-8859-1
$ echo hi > f && git add f && printf 'caf\xe9\n' | git commit -q -S -F -
$ git fast-export --signed-commits=verbatim --reencode=no --all > ../signed.stream
```

git 2.56.0 writes `gpgsig sha1 ssh`, its `data`, then `encoding ISO-8859-1`.
`Reader::read` on that stream returns the blob and the reset, then:

```text
line 19: expected `data`, found `encoding`
```

The other direction: move the `encoding` line above `gpgsig`, which is the
order `Writer` writes, and feed the result to git:

```console
$ git init -q t && cd t && git fast-import --quiet < ../swapped.stream
fatal: expected 'data n' command, found: gpgsig sha1 ssh
```

## What it costs

The command line is not affected. `import` runs `git fast-export
--reencode=yes`, which converts the message to UTF-8 and leaves out
`encoding`. `write` always sets `encoding: None`. The bug is in the public
`stream` module, which says it reads "what `git fast-export` writes" and
writes what `git fast-import` reads. With both headers present, it does
neither.

## Done when

`Reader` reads `gpgsig` before `encoding` and `Writer` writes them in that
order. A test reads a stream that has both, in the shape git writes, and
writes it back byte for byte. The comment at `read.rs` line 160 describes
git's order.
