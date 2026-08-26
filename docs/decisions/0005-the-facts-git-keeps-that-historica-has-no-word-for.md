# 0005 — The facts git keeps that historica has no word for

Decision 0004 measured its own gap. A commit is a function of the revision it
came from, so the round trip is an identity — except for the git facts this
format has no word for, which were left under Deferred waiting on a version of
historica that could state one:

> Carrying the committer, the encoding, and the signature across, which would
> shrink the uncarried set to nothing. Historica's decision 0065 has the
> mechanism […] but `record::Recording` has no way to state one today.

Historica's decision 0070 built the field. This decides what goes in it.

## The decision

- **Two facts cross, under keys this tool owns.** `git.committer` is the whole
  committer line, `Name <email> 1767348245 -0700`, written only where the commit
  named a committer other than its author — where it did not, decision 0004's
  restated author already reproduces the line. `git.signature` is the signature
  and `git.signature-kind` is what `gpgsig` said it was, as two keys rather than
  one value with a grammar in it.

- **The encoding does not, and it is not the same kind of gap.** The others are
  facts *beside* the message; an `encoding` is a claim *about* it, and the
  conversion has already re-encoded the message to UTF-8 by the time it is
  recorded, because historica's documents are UTF-8 and a revision cannot hold
  the original bytes. Carrying the declaration without the bytes it describes
  would produce a commit that says it is Latin-1 and is not, which is worse than
  one that says nothing. So it stays reported, and closing it is a question
  about where a non-UTF-8 message lives rather than about headers.

- **A header value is spelled with the stream's own quoting.** Historica's rule
  for any value is one line, no control characters, no leading or trailing
  space. A committer satisfies it as it stands and stays readable in the
  document; an armoured signature has newlines and does not. Rather than invent
  an escape, this uses the one this crate already has — `stream::quote`, which
  git's own paths travel under — so the escape a person meets in a revision
  document is the escape they already met in the stream, and there is one rule
  to get wrong instead of two. `git.signature` opens with a legible
  `"-----BEGIN PGP SIGNATURE-----` for the same reason base64 was not chosen:
  a person reading the file can see what it is.

- **The spelling is a function of the bytes, not a choice about them.** A header
  value is in the revision's canonical bytes, so two runs of a conversion that
  spelled one signature two ways would write two revision IDs — and decision
  0003's reproducibility would fail only when somebody re-imported, which is the
  worst time to find out. So `spell` is pure, and a test sweeps every byte value
  asserting that it round-trips, that it is spelled the same way twice, and that
  what comes out is ASCII, which is what makes the final `from_utf8_lossy`
  lossless rather than lucky.

- **A name ending in a space loses it, as it would in git.** The separator
  before the address is a space, so a trailing space in a name is one git's own
  reader cannot tell from the separator; it trims, and git normalises such a
  name away before a commit object holds one. It is not a fact dropped between
  the two, either: historica's `split_header` refuses a padded value outright on
  its decision 0002's rule that a value must survive a round trip, so neither
  end can represent such a person and the trim discards nothing the other side
  could have held. Stated because it is the one input the committer round trip
  does not survive, and an unstated exception is a surprise.

- **A signature does not survive an amendment.** Historica's 0023 carries a
  header across a rewrite because a writer that cannot read one must not drop
  it, and that is right for every header except this one. A signature is a claim
  about exact bytes, so a revision that supersedes another has no business
  carrying its predecessor's — 0070 says it in as many words: a rewritten
  commit's signature is not stale, it is wrong. So a conversion writes no
  `gpgsig` for a revision whose `supersedes` is not empty, and says how many it
  withheld. The committer is kept, because a committer is a name rather than a
  claim, and at worst it is imprecise.

- **A header this tool cannot read is reported, not guessed at.** Something else
  may write under a `git.` key — 0065 refused a registry and 0070 kept refusing
  it — so a value that does not say what this tool writes is named and skipped.

## Why the signature is worth the weight it adds

It is most of the gap, and the measurement is the argument. Pointed at
historica-git's own repository before this: eighteen commits, every one signed,
none round-tripping. After it: all eighteen, and `git log --show-signature`
calls them good in the repository that was written back — which is a stronger
check than comparing object IDs, because a signature verifies over the bytes
rather than over a name for them.

A repository whose commits are signed is not an unusual repository. It is most
of the ones anybody would want to drive git with.

## Consequences

- `import` asks for `--signed-commits=verbatim` rather than `warn-strip`, so the
  signature reaches the stream instead of a warning reaching stderr. The stream
  model gained `gpgsig`, both ways, which the reader and the writer are held to
  as one rule like everything else there.
- The uncarried set from this end is now the encoding, a submodule, and an
  annotated tag. None of the three is a header that would fix it.
- A conversion of a store that has been amended since it was imported writes
  commits that are honestly unsigned rather than dishonestly signed, and the
  count is in the report.

## Deferred

- **The encoding**, above, and with it what a non-UTF-8 message is.
- **Whether `gpgsig` and `encoding` are written in the order git writes them**
  when a commit carries both. The corpus has no such commit and neither has any
  repository this has been pointed at, so the order here is the observed one for
  each separately and an assumption where they meet.
- **Signing on the way out.** Nothing here makes a signature; it moves one.
