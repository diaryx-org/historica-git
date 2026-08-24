#!/bin/sh
# Rebuild `all.fi` from the repository it is an export of.
#
# The fixture is checked in rather than built during the test run, because a
# test that builds its own input tests the machine it runs on as much as the
# code. This script is here so the fixture can be regenerated when git's output
# changes or the history needs another case in it — run it, read the diff, and
# commit that diff deliberately.
#
# Everything that could vary is pinned: the dates, the identities, the branch
# name, and signing, which is off because a signature is not reproducible and
# `git fast-export` refuses a signed tag outright unless told what to do with
# one.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"

export GIT_AUTHOR_NAME=Ada GIT_AUTHOR_EMAIL=ada@example.com
export GIT_COMMITTER_NAME=Bo GIT_COMMITTER_EMAIL=bo@example.com
export GIT_AUTHOR_DATE="2026-01-02T03:04:05-07:00"
export GIT_COMMITTER_DATE="2026-01-02T03:04:06-07:00"

git init -qb main .
git config commit.gpgsign false
git config tag.gpgsign false

# A text file, a file of bytes that is not text, and a path holding a space,
# which git quotes and this format's readers therefore have to unquote.
printf 'one\ntwo\n' > a.txt
printf '\000\001\002binary\n' > photo.bin
mkdir -p sub && printf 'nested\n' > "sub/with space.txt"
git add -A && git commit -qm "Start"

# An edit, the executable bit, and a symlink — the two modes beyond 100644 that
# an ordinary repository has.
printf 'one\ntwo\nthree\n' > a.txt
chmod +x a.txt
ln -s a.txt link
git add -A && git commit -qm "Edit and link"

# A rename, which `-M` states as `R` rather than as a delete and an add, and a
# delete beside it.
git mv a.txt renamed.txt
git rm -q photo.bin
git commit -qm "Rename and delete"

# A second line of work and a merge, so the stream has a commit with two
# parents in it.
git checkout -q -b side HEAD~2
printf 'side\n' > side.txt
git add -A && git commit -qm "Side"
git checkout -q main
git merge -q --no-ff side -m "Merge side"

# A commit that changed nothing, a lightweight tag, and an annotated one.
git commit -q --allow-empty -m "Empty"
git tag light
git tag -a annotated -m "An annotated tag"

git fast-export --all -M > "$here/all.fi"
cd "$here" && shasum -a 256 all.fi invalid/*.fi > MANIFEST
echo "wrote $here/all.fi"
