# 0003 — A change is the commit it came from

`historica::record` mints a change ID from 96 bits of entropy, and its decision
0001 is why: a change ID names the work rather than the content, so it must
survive amendment and rebase, and every derivation from the revision's own
content or parents changes under exactly those two operations.

For a conversion that is the wrong end of the argument. Import a repository on
one machine and again on another and nothing matches: different change IDs make
different revision bytes, different bytes make different digests, and `receive`
unions two copies of one history because it has no way to see they are one. The
same repository converted twice should be the same history, and with minted IDs
it never is.

## The decision

- **A change ID is the first 96 bits of the git commit's object ID.** The same
  commit converts to the same change everywhere, by anyone, at any time.

- **The export is run with `--show-original-ids`, and a stream without them is
  refused.** That flag is the only way the object ID reaches the stream; a
  conversion that fell back to minting would produce a store that looks right
  and converges with nothing.

- **File identifiers are derived too**, from the commit's object ID and the
  order historica asks for them, because a file ID is written into the
  revision's bytes and a random one would undo everything above.

- **Everything git carries that historica has no place for is dropped, and
  said.** The committer, signatures, the commit encoding, notes, and the object
  ID itself do not cross. Decision 0001 requires a conversion to state what it
  could not carry rather than drop it quietly, and that statement is where
  those facts go until there is somewhere better.

## Why this does not contradict historica's 0001

That decision rejected two derivations, and both rejections were about the same
thing: an ID that moves when the work is rewritten. `patch-id` moves under
amendment and rebase; a digest of the revision's own bytes is circular.

A git commit's object ID is neither. It is a name fixed before historica ever
saw the work, by a tool that is not going to revise it, and it does not move
when the imported revision is later amended — the amendment carries the change
ID forward, which is precisely the job change IDs exist to do. So the property
0001 protects is intact: the change ID still names the work across every
rewrite historica performs on it.

What it does mean is that a commit **rebased upstream** re-imports as a
different change. That is correct rather than unfortunate: a rebased commit is
a different commit, and git has already thrown away the fact that it was ever
the other one.

## Read across by hand

historica spells an assigned identifier in reversed hexadecimal — nibble 0 is
`z` and nibble 15 is `k`, so that no change ID can be mistaken for a digest.
The bytes underneath are the object ID's own, so `git log` and `historica log`
line up with no tool between them:

```
980aeeabdf5024a43392620b11f1d14de03a0bb5   the commit
980aeeabdf5024a43392620b                   its first twelve bytes
qrzpllpomkuzxvpvwwqxtxzo                   the same, as historica spells it
```

That is decision 0001's rule about checking by hand, met without a command.

## Consequences

- A repository whose objects are SHA-256 converts to different change IDs than
  the same history under SHA-1. Two spellings of one commit are two names, and
  nothing can recover one from the other.
- Two commits sharing an object ID share a change ID, which is right: they are
  the same commit.
- Conversion is reproducible, so a conversion can be *re-run* rather than
  resumed, and `receive` will fold the second run into the first.

## Deferred

- **File IDs by path.** They are drawn in the order historica asks, which is
  the sorted order of `Survey::added`. Deriving each from the object ID and the
  path it is minted for would remove that coupling, and `survey` is public, so
  the conversion can do it once it calls `survey` on its own account.
- Where the dropped facts are written down, which is 0001's open question about
  the correspondence file and not this decision's to answer.
