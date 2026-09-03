---
title: The run around every historica command
description: 0008 defers the thing that makes colocation worth having — keeping `.git/` current after a historica command wrote — and historica's 0074 is the format it can be triggered from
status: open
created: 2026-09-02
updated: 2026-09-02
part_of: "[Tasks](tasks.md)"
---

# The run around every historica command

[0008](../decisions/0008-one-directory-and-git-derived-in-it.md) defers it by
name, and says what it is worth:

> **The run around every historica command**, which is what makes colocation
> worth having and what detection will report through. Its own decision, and
> the next one.

Colocation as it stands gives a person one directory in which `git log`, `git
blame` and an editor's gutter are correct — but only as of the last time
somebody ran a write by hand. The half that is missing is the *when*: what makes
the projection in `.git/` current after `historica record` or `historica
receive` has put something in the store.

## What 0074 changes about it

Historica's decision 0074 adds `historica-wrote-1`: a writing command run with
`--fields` prints a statement of what it wrote, and prints a header with no
lines when it wrote nothing. That is the trigger this deferral was waiting for,
and it settles two things the decision would otherwise have to invent.

- **The signal is a pipe, not a hook.** Historica's 0053 and 0072 refuse
  dispatch out of `historica` into tools beside it, so nothing here can be made
  to run *by* `historica record`. What can happen is that a person's alias or a
  wrapper reads the statement:

  ```sh
  historica record --fields -m 'note' | historica-git write --wrote -
  ```

- **Wrote nothing costs nothing.** An empty statement means the wrapper
  returns without opening the store, which is what makes putting this on every
  command bearable.

The conversion itself is already the cheap half: 0007 made `write` incremental
onto what it made before, so what this adds is the trigger and not a new
conversion.

## The work

- **A decision first.** 0008 says this is its own, and it is: what the run does
  when the statement names revisions the projection has not seen, what it does
  when detection finds a ref that moved on its own in the same breath, and
  whether the run is `write` with a flag or a command of its own. The stance to
  argue it under is 0008's — historica owns the directory and git is derived in
  it — which is the reason this is a write after the fact rather than jj's
  import before every command.
- **Read the statement with historica's parser**, not a second implementation
  of the grammar. Historica's 0053 says a tool beside it takes what it needs
  from the API, and that parser is a task in historica. This is blocked on it.
- **A statement with a line kind this build does not know is discarded whole**,
  per 0074, which means the run refuses rather than projecting part of it.
- **Say what a person types.** Whatever the shape, the README has to show the
  one-line alias, because the deferral is only closed when somebody's ordinary
  day has git current in it without them thinking about it.

## Done when

- A colocated directory in which every historica command is run through the
  wrapper has a `.git/` that is current after each one, with no reconciliation
  paid on a command that wrote nothing.
- Detection still reports a ref that moved on its own, and the run and the
  detection do not contradict each other in the same command.
- The decision is written and 0008's deferral names it.
