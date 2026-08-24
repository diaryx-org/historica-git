//! Cutting a release, as one program.
//!
//! historigit releases on a tag: pushing `vX.Y.Z` starts `release.yml`, which
//! cuts the GitHub release and writes its body from the changelog. Everything
//! before that push is mechanical and easy to get half-right by hand — the
//! version lives in the manifest and the lockfile both, and the changelog's
//! unreleased region has to be cut into a released section — so it lives here
//! instead:
//!
//!     cargo xtask version                 what the repository calls itself
//!     cargo xtask bump <patch|minor|major|X.Y.Z>
//!     cargo xtask changelog [--write|--check]
//!     cargo xtask release <patch|minor|major|X.Y.Z> [--push] [--no-verify]
//!     cargo xtask release-notes [tag]
//!
//! `release` stops at the tag unless it is given `--push`. That asymmetry is the
//! whole safety model: every step before the push is a local commit that can be
//! amended or thrown away, and the push is the one that puts a version number
//! somewhere other people can see it. So the push is asked for explicitly, each
//! time, and the default run prints the two commands it did not run.
//!
//! `release-notes` is what `release.yml` invokes, so the release workflow holds
//! no more knowledge about this repository than the CI workflow does: it asks
//! the program.

use std::fmt;

use crate::{Result, Sh};

/// The changelog, and the config that generates half of it.
const CHANGELOG: &str = "docs/CHANGELOG.md";
const CLIFF_CONFIG: &str = ".config/cliff.toml";

/// The generated region inside `## Unreleased`. Only the bytes between these
/// two lines are ever rewritten; a handwritten release intro lives below the
/// end marker, in the released section, where regeneration cannot reach it.
const BEGIN: &str = "<!-- git-cliff:begin — generated; edits here are overwritten -->";
const END: &str = "<!-- git-cliff:end -->";
/// What the region says when there is nothing unreleased — the normal state
/// immediately after a release.
const EMPTY_REGION: &str = "_No commits since the last tag._";

/// Where a reader is sent for the rest of the history, from a release body.
const REPO: &str = "https://github.com/diaryx-org/historigit";

// ---------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------

/// A semver triple, which is all this repository uses. Pre-release and build
/// metadata are deliberately unparsed rather than silently dropped: a version
/// this cannot read is a version it must not rewrite.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    fn parse(text: &str) -> Result<Self> {
        let mut parts = text.trim().split('.');
        let mut next = || -> Result<u64> {
            parts
                .next()
                .and_then(|p| p.parse().ok())
                .ok_or_else(|| format!("`{text}` is not an x.y.z version"))
        };
        let version = Version {
            major: next()?,
            minor: next()?,
            patch: next()?,
        };
        match parts.next() {
            None => Ok(version),
            Some(_) => Err(format!("`{text}` is not an x.y.z version")),
        }
    }

    /// `patch`, `minor`, `major`, or a literal version to move to. A literal is
    /// checked against the current version rather than trusted: a release that
    /// goes backwards is a typo every time, and the tag it would cut is the one
    /// thing that cannot be taken back.
    fn bump(self, spec: &str) -> Result<Self> {
        match spec {
            "patch" => Ok(Version {
                patch: self.patch + 1,
                ..self
            }),
            "minor" => Ok(Version {
                minor: self.minor + 1,
                patch: 0,
                ..self
            }),
            "major" => Ok(Version {
                major: self.major + 1,
                minor: 0,
                patch: 0,
            }),
            literal => {
                let next = Version::parse(literal)?;
                if next.ordered() <= self.ordered() {
                    return Err(format!(
                        "{next} is not ahead of the current {self}\n\
                         hint: releases only move forward — a tag that has been pushed is one \
                         other people have already fetched",
                    ));
                }
                Ok(next)
            }
        }
    }

    fn ordered(self) -> (u64, u64, u64) {
        (self.major, self.minor, self.patch)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// `workspace.package.version` — the version the package inherits, and the one
/// the release workflow compares the tag against.
fn workspace_version(sh: &Sh) -> Result<Version> {
    let manifest = sh.read("Cargo.toml")?;
    let line = manifest
        .lines()
        .find(|line| line.starts_with("version = \""))
        .ok_or_else(|| "no `version` in [workspace.package]".to_string())?;
    Version::parse(line.split('"').nth(1).unwrap_or_default())
}

pub fn print_version(sh: &Sh) -> Result<()> {
    println!("{}", workspace_version(sh)?);
    Ok(())
}

/// Move the repository to `next`.
///
/// One rewrite — `[workspace.package] version` — and then the lockfile, which
/// records the members' own versions and so moves with them. `--workspace`
/// touches nothing else: a release is not the moment to pick up a new upstream
/// dependency.
///
/// The count is checked rather than assumed. `[package]` inherits the value with
/// `version.workspace = true`, so exactly one line in the manifest holds it; a
/// second one would mean the two could disagree, and rewriting only the first
/// would ship the disagreement.
fn set_version(sh: &Sh, next: Version) -> Result<()> {
    let manifest = sh.read("Cargo.toml")?;
    let mut out = String::with_capacity(manifest.len());
    let mut found = 0;

    for line in manifest.lines() {
        if line.starts_with("version = \"") {
            out.push_str(&format!("version = \"{next}\""));
            found += 1;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }

    if found != 1 {
        return Err(format!(
            "expected exactly one `version = \"…\"` line in Cargo.toml, found {found}"
        ));
    }
    sh.write("Cargo.toml", &out)?;
    println!("Cargo.toml -> {next}");

    sh.cargo(&["update", "--workspace", "--quiet"])
}

pub fn bump(sh: &Sh, spec: &str) -> Result<()> {
    let current = workspace_version(sh)?;
    let next = current.bump(spec)?;
    println!("{current} -> {next}");
    set_version(sh, next)
}

// ---------------------------------------------------------------------------
// The changelog
// ---------------------------------------------------------------------------

/// The unreleased commits, rendered by git-cliff through `.config/cliff.toml`.
///
/// git-cliff exits non-zero when there is nothing unreleased, which is a normal
/// state right after a tag rather than a failure — hence the placeholder rather
/// than an error.
fn generated(sh: &Sh) -> Result<String> {
    sh.require(
        "git-cliff",
        "nix profile install nixpkgs#git-cliff, or cargo install git-cliff",
    )?;
    let rendered = sh
        .capture(
            "git-cliff",
            &["--config", CLIFF_CONFIG, "--unreleased", "--strip", "all"],
        )
        .unwrap_or_default();
    let body = rendered.trim();
    Ok(if body.is_empty() {
        EMPTY_REGION.to_string()
    } else {
        body.to_string()
    })
}

/// The commits a tag covers, rendered the same way the unreleased region is.
///
/// The range starts at the previous tag, or — for the first tag ever — at the
/// repository's root commit, which is the closest thing to "before everything"
/// that git-cliff will accept as a range.
fn tagged(sh: &Sh, previous: Option<&str>, tag: &str) -> Result<String> {
    let root;
    let start = match previous {
        Some(previous) => previous,
        None => {
            root = sh.capture("git", &["rev-list", "--max-parents=0", "HEAD"])?;
            root.trim()
        }
    };
    let rendered = sh
        .capture(
            "git-cliff",
            &[
                "--config",
                CLIFF_CONFIG,
                "--strip",
                "all",
                &format!("{start}..{tag}"),
            ],
        )
        .unwrap_or_default();
    let body = rendered.trim();
    Ok(if body.is_empty() {
        "_Nothing recorded._".to_string()
    } else {
        body.to_string()
    })
}

/// Every `v*` tag, oldest first — the same pattern `.config/cliff.toml` sections
/// history by.
fn tags(sh: &Sh) -> Result<Vec<String>> {
    Ok(sh
        .capture("git", &["tag", "--sort=v:refname", "--list", "v[0-9]*"])?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Tags with no section of their own, rendered and dated, newest first.
///
/// A tag can appear after the fact — cut long after the commits it names, once
/// the release it stood for had already happened — and without this, those
/// commits would simply leave the file: no longer unreleased, and in no section
/// either. So the tag list, not the last write, decides what the changelog owes.
fn missing_sections(sh: &Sh, text: &str) -> Result<Vec<String>> {
    let tags = tags(sh)?;
    let mut sections = Vec::new();
    for (index, tag) in tags.iter().enumerate() {
        if text
            .lines()
            .any(|line| section_tag(line) == Some(tag.as_str()))
        {
            continue;
        }
        let previous = index.checked_sub(1).map(|i| tags[i].as_str());
        let body = tagged(sh, previous, tag)?;
        let date = sh.capture("git", &["log", "-1", "--format=%cs", tag])?;
        sections.push(format!("## {tag} — {}\n\n{body}\n", date.trim()));
    }
    Ok(sections)
}

/// `## v0.2.0 — 2026-08-21` -> `v0.2.0`, and anything else -> `None`.
fn section_tag(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("## ")?;
    let tag = rest.split_whitespace().next()?;
    tag.strip_prefix('v')
        .filter(|version| version.starts_with(|c: char| c.is_ascii_digit()))
        .map(|_| tag)
}

/// Put sections in their place: newest first, and an older one folded in above
/// the first section it outranks rather than appended to the end.
fn insert_sections(text: &str, sections: Vec<String>) -> String {
    if sections.is_empty() {
        return text.to_string();
    }
    let order = |tag: &str| Version::parse(tag.trim_start_matches('v')).ok();

    let mut out = String::with_capacity(text.len());
    let mut pending: Vec<String> = sections;
    for line in text.lines() {
        if let Some(tag) = section_tag(line)
            && let Some(here) = order(tag)
        {
            // Everything newer than the section starting on this line goes in
            // ahead of it.
            let (newer, rest): (Vec<String>, Vec<String>) = pending.into_iter().partition(|s| {
                section_tag(s.lines().next().unwrap_or_default())
                    .and_then(order)
                    .is_some_and(|candidate| candidate.ordered() > here.ordered())
            });
            for section in newer {
                out.push_str(&section);
                out.push('\n');
            }
            pending = rest;
        }
        out.push_str(line);
        out.push('\n');
    }
    // Whatever is older than every section already in the file lands at the end.
    for section in pending {
        out.push_str(&section);
        out.push('\n');
    }
    out
}

fn region(body: &str) -> String {
    format!("{BEGIN}\n\n{body}\n\n{END}")
}

/// Replace the marked region, and optionally drop a fresh released section in
/// immediately below it. Everything above `## Unreleased` and every released
/// section below is left byte-for-byte alone.
fn rewrite(text: &str, body: &str, released: Option<&str>) -> Result<String> {
    for marker in [BEGIN, END] {
        if !text.lines().any(|line| line == marker) {
            return Err(format!("marker not found in {CHANGELOG}:\n  {marker}"));
        }
    }

    let mut out = String::with_capacity(text.len());
    let mut skipping = false;
    for line in text.lines() {
        if line == BEGIN {
            out.push_str(&region(body));
            out.push('\n');
            skipping = true;
        } else if line == END {
            skipping = false;
            if let Some(section) = released {
                out.push('\n');
                out.push_str(section);
                out.push('\n');
            }
        } else if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
}

/// `cargo xtask changelog [--write|--check]` — print, splice, or verify.
pub fn changelog(sh: &Sh, args: &[&str]) -> Result<()> {
    let mode = match args {
        [] => "print",
        ["--write"] => "write",
        ["--check"] => "check",
        _ => return Err("usage: cargo xtask changelog [--write|--check]".into()),
    };

    let body = generated(sh)?;
    if mode == "print" {
        println!("{}", region(&body));
        return Ok(());
    }

    let current = sh.read(CHANGELOG)?;
    // Two jobs, because a tag can arrive after the commits it names: refresh the
    // unreleased region, and give any tag still without a section one.
    let missing = missing_sections(sh, &current)?;
    let named: Vec<String> = missing
        .iter()
        .filter_map(|s| section_tag(s.lines().next().unwrap_or_default()).map(str::to_owned))
        .collect();
    let spliced = insert_sections(&rewrite(&current, &body, None)?, missing);

    if mode == "check" {
        if !named.is_empty() {
            return Err(format!(
                "{CHANGELOG} has no section for {}\nhint: run `cargo xtask changelog --write`",
                named.join(", ")
            ));
        }
        if spliced != current {
            return Err(format!(
                "{CHANGELOG}'s generated region is stale\nhint: run `cargo xtask changelog --write`"
            ));
        }
        println!("{CHANGELOG}'s generated region is up to date");
        return Ok(());
    }

    sh.write(CHANGELOG, &spliced)?;
    for tag in &named {
        println!("{CHANGELOG} -> new section `## {tag}`");
    }
    println!("wrote {CHANGELOG}");
    Ok(())
}

/// One release's section of the changelog, without its heading — the body of a
/// GitHub release, for the notes job in `release.yml`.
///
/// Read from `docs/CHANGELOG.md` rather than rendered from the commits, and
/// deliberately: the tag's tree already carries the section `release` cut, so
/// the notes a reader finds on the release page are byte-identical to the ones
/// the repository ships, and the runner needs no git-cliff to produce them.
pub fn release_notes(sh: &Sh, tag: Option<&str>) -> Result<()> {
    let tag = match tag {
        Some(tag) => tag.to_string(),
        None => format!("v{}", workspace_version(sh)?),
    };
    let changelog = sh.read(CHANGELOG)?;
    let body = section(&changelog, &tag).ok_or_else(|| {
        format!(
            "{CHANGELOG} has no section for {tag}\n\
             hint: `cargo xtask changelog --write`, commit, and re-run — a tag whose \
             changelog section is missing is a tag cut without one"
        )
    })?;

    println!("{body}");
    println!(
        "\n---\n\n\
         Every change in this release and the ones before it: \
         [docs/CHANGELOG.md]({REPO}/blob/{tag}/docs/CHANGELOG.md)."
    );
    Ok(())
}

/// The lines under `## <tag> — …`, up to the next `##` heading. A `###` group
/// heading is part of the section, so the search is for that heading level
/// exactly — and so a handwritten release intro, which sits directly under the
/// heading, comes along with the generated groups.
fn section(changelog: &str, tag: &str) -> Option<String> {
    let mut lines = changelog
        .lines()
        .skip_while(|line| section_tag(line) != Some(tag));
    lines.next()?;
    let body: Vec<&str> = lines.take_while(|line| !line.starts_with("## ")).collect();
    Some(body.join("\n").trim().to_string())
}

/// Turn the unreleased region into a released section headed `## vX.Y.Z — date`,
/// and reset the region. Called by `release`, between the version bump and the
/// commit, so the release commit carries both.
fn cut_changelog(sh: &Sh, version: Version) -> Result<()> {
    let body = generated(sh)?;
    let date = sh.capture("date", &["+%Y-%m-%d"])?.trim().to_string();
    let released = format!("## v{version} — {date}\n\n{body}\n");
    let current = sh.read(CHANGELOG)?;
    let cut = rewrite(&current, EMPTY_REGION, Some(&released))?;
    sh.write(CHANGELOG, &cut)?;
    println!("{CHANGELOG} -> new section `## v{version} — {date}`");
    Ok(())
}

// ---------------------------------------------------------------------------
// Releasing
// ---------------------------------------------------------------------------

/// `cargo xtask release <patch|minor|major|X.Y.Z> [--push] [--no-verify]`.
///
/// Bump, regenerate, commit, tag — and push only when asked. What the push
/// starts is worth stating plainly: `release.yml` cuts a GitHub release at that
/// tag and writes its body from the changelog section this command just wrote.
/// A release can be deleted, but a tag other people have already fetched cannot
/// be taken back from them.
pub fn release(sh: &Sh, spec: &str, args: &[&str]) -> Result<()> {
    let (mut push, mut verify) = (false, true);
    for arg in args {
        match *arg {
            "--push" => push = true,
            "--no-verify" => verify = false,
            other => {
                return Err(format!(
                    "unknown option `{other}`\n\
                     usage: cargo xtask release <patch|minor|major|X.Y.Z> [--push] [--no-verify]"
                ));
            }
        }
    }

    let current = workspace_version(sh)?;
    let next = current.bump(spec)?;
    let tag = format!("v{next}");

    // Everything that can say "no" says it before anything is written. A
    // half-applied release is a working tree to untangle by hand, and the whole
    // point of this command is not doing that.
    preflight(sh, &tag)?;

    if verify {
        println!("\n\x1b[1m━━ CI ━━\x1b[0m");
        crate::ci(sh)?;
    } else {
        println!("skipping CI (--no-verify)");
    }

    println!("\n\x1b[1m━━ {current} -> {next} ━━\x1b[0m");
    set_version(sh, next)?;
    cut_changelog(sh, next)?;

    // Only the three files a release moves, named explicitly: whatever else is
    // in the tree stays out of the release commit.
    sh.run("git", &["add", "Cargo.toml", "Cargo.lock", CHANGELOG])?;
    sh.run("git", &["commit", "-m", &format!("chore: bump to {next}")])?;
    // Annotated: the release workflow reads `github.ref_name`, and
    // `git describe` wants an object to read.
    sh.run("git", &["tag", "-a", &tag, "-m", &tag])?;

    let branch = sh.capture("git", &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = branch.trim().to_string();

    if !push {
        println!(
            "\n\x1b[32m{tag} is committed and tagged locally.\x1b[0m\n\n\
             Nothing has left this machine. To release:\n\n    \
             git push origin {branch}\n    \
             git push origin {tag}\n\n\
             The tag is what publishes: `release.yml` cuts the GitHub release at {tag}\n\
             and writes its body from the changelog section above.\n\n\
             To undo locally instead: git tag -d {tag} && git reset --hard HEAD~1\n"
        );
        return Ok(());
    }

    sh.run("git", &["push", "origin", &branch])?;
    sh.run("git", &["push", "origin", &tag])?;
    println!("\n\x1b[32m{tag} pushed.\x1b[0m The release is running:\n    {REPO}/actions\n");
    Ok(())
}

/// Refuse a release that is already doomed: dirty tree, wrong branch, a tag that
/// exists, or no git-cliff to write the changelog with.
fn preflight(sh: &Sh, tag: &str) -> Result<()> {
    sh.require(
        "git-cliff",
        "nix profile install nixpkgs#git-cliff, or cargo install git-cliff",
    )?;

    if !sh
        .capture("git", &["status", "--porcelain"])?
        .trim()
        .is_empty()
    {
        return Err(
            "the working tree is dirty — commit or stash first, so the release commit holds \
             only the version bump and the changelog"
                .into(),
        );
    }

    let branch = sh.capture("git", &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch.trim() != "main" {
        return Err(format!(
            "on branch `{}`, and historigit releases from `main`",
            branch.trim()
        ));
    }

    if !sh
        .capture("git", &["tag", "--list", tag])?
        .trim()
        .is_empty()
    {
        return Err(format!("tag {tag} already exists locally"));
    }
    if !sh
        .capture("git", &["ls-remote", "--tags", "origin", tag])?
        .trim()
        .is_empty()
    {
        return Err(format!("tag {tag} already exists on origin"));
    }

    // A release cut on a stale main is a release missing commits. Fetch is
    // advisory — a laptop offline enough to fail it can still cut the local
    // commit and push later.
    if sh
        .capture("git", &["fetch", "--quiet", "origin", "main"])
        .is_ok()
    {
        let behind = sh.capture("git", &["rev-list", "--count", "HEAD..origin/main"])?;
        if behind.trim() != "0" {
            return Err(format!(
                "main is {} commits behind origin/main — pull first",
                behind.trim()
            ));
        }
    } else {
        println!("warning: could not reach origin; releasing against the local main");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_move_forward_only() {
        let current = Version::parse("0.1.0").unwrap();
        assert_eq!(current.bump("patch").unwrap().to_string(), "0.1.1");
        assert_eq!(current.bump("minor").unwrap().to_string(), "0.2.0");
        assert_eq!(current.bump("major").unwrap().to_string(), "1.0.0");
        assert_eq!(current.bump("0.9.3").unwrap().to_string(), "0.9.3");
        assert!(current.bump("0.0.9").is_err(), "a release cannot go back");
        assert!(current.bump("0.1.0").is_err(), "nor stand still");
        assert!(current.bump("0.1").is_err());
        assert!(
            current.bump("0.1.0-rc.1").is_err(),
            "unparsed, not truncated"
        );
    }

    /// The bump rewrites one line, and the manifest has to be the shape that
    /// makes that safe: the version stated once, in `[workspace.package]`, and
    /// inherited by `[package]` rather than repeated there.
    #[test]
    fn the_manifest_states_its_version_exactly_once() {
        let sh = Sh::new();
        let manifest = sh.read("Cargo.toml").unwrap();
        assert_eq!(
            manifest
                .lines()
                .filter(|line| line.starts_with("version = \""))
                .count(),
            1,
        );
        assert!(manifest.contains("version.workspace = true"));
        // And that line is the one `cargo xtask version` reads.
        let version = workspace_version(&sh).unwrap();
        assert!(manifest.contains(&format!("version = \"{version}\"")));
    }

    /// The splice touches the region and nothing else — not the prose above it,
    /// and not a single released section below.
    #[test]
    fn rewriting_leaves_everything_outside_the_region_alone() {
        let text = format!(
            "# Changelog\n\nprose\n\n## Unreleased\n\n{BEGIN}\n\nold\n\n{END}\n\n## v0.1.0 — 2026-08-07\n\nkept\n"
        );
        let refreshed = rewrite(&text, "new", None).unwrap();
        assert!(refreshed.contains("\nnew\n") && !refreshed.contains("\nold\n"));
        assert!(refreshed.contains("# Changelog\n\nprose"));
        assert!(refreshed.ends_with("## v0.1.0 — 2026-08-07\n\nkept\n"));

        let cut = rewrite(&text, EMPTY_REGION, Some("## v0.2.0 — 2026-08-21\n\nnew\n")).unwrap();
        let released = cut.find("## v0.2.0").unwrap();
        assert!(
            released > cut.find(END).unwrap(),
            "released sections go below the region"
        );
        assert!(
            released < cut.find("## v0.1.0").unwrap(),
            "newest release first"
        );
        assert!(cut.contains(EMPTY_REGION));
    }

    /// A tag section is recognised by its heading and nothing else, so that a
    /// handwritten intro under one is never mistaken for a heading — and a
    /// heading that is prose (`## Unreleased`) is never mistaken for a tag.
    #[test]
    fn tag_headings_are_told_from_prose_ones() {
        assert_eq!(section_tag("## v0.1.0 — 2026-08-21"), Some("v0.1.0"));
        assert_eq!(section_tag("## v0.1.0"), Some("v0.1.0"));
        assert_eq!(section_tag("## Unreleased"), None);
        assert_eq!(section_tag("## verify the build"), None);
        assert_eq!(section_tag("### Added"), None);
        assert_eq!(section_tag("- **cli** — ## v1.0.0 in a bullet"), None);
    }

    /// A tag can be cut long after the commits it names, so a backfilled section
    /// has to land in version order rather than on top.
    #[test]
    fn backfilled_sections_land_in_version_order() {
        let text = "## v0.6.0 — c\n\nsix\n\n## v0.4.0 — a\n\nfour\n";
        let merged = insert_sections(
            text,
            vec![
                "## v0.5.0 — b\n\nfive\n".to_string(),
                "## v0.3.0 — z\n\nthree\n".to_string(),
            ],
        );
        let order: Vec<&str> = merged.lines().filter_map(section_tag).collect();
        assert_eq!(order, ["v0.6.0", "v0.5.0", "v0.4.0", "v0.3.0"]);
        // Nothing already in the file is rewritten on the way past.
        for kept in ["six", "four"] {
            assert_eq!(merged.matches(kept).count(), 1);
        }
    }

    #[test]
    fn nothing_missing_leaves_the_file_untouched() {
        let text = "## v0.6.0 — c\n\nsix\n";
        assert_eq!(insert_sections(text, vec![]), text);
    }

    /// The release body is a slice of the changelog, so the slice has to end
    /// where the next release begins — and not at a group heading inside it.
    #[test]
    fn a_release_section_stops_at_the_next_release() {
        let changelog = "\
# Changelog

## Unreleased

nothing

## v0.2.0 — 2026-08-21

An intro, handwritten.

### Added

- a thing

## v0.1.0 — 2026-08-06

### Added

- an older thing
";
        let body = section(changelog, "v0.2.0").unwrap();
        assert!(body.starts_with("An intro, handwritten."), "{body}");
        assert!(body.contains("### Added") && body.contains("- a thing"));
        assert!(
            !body.contains("older"),
            "the next release is not part of it"
        );
        assert!(!body.contains("## v0.1.0"));

        assert_eq!(
            section(changelog, "v0.1.0").unwrap(),
            "### Added\n\n- an older thing"
        );
        assert_eq!(section(changelog, "v9.9.9"), None, "a tag with no section");
    }

    #[test]
    fn rewriting_a_changelog_without_markers_is_an_error() {
        assert!(rewrite("# Changelog\n", "new", None).is_err());
    }

    /// The markers are the whole contract between this file and the changelog:
    /// if the committed file loses one, every `--write` and `--check` fails.
    #[test]
    fn the_committed_changelog_has_its_markers() {
        let text = Sh::new().read(CHANGELOG).unwrap();
        for marker in [BEGIN, END] {
            assert!(
                text.lines().any(|line| line == marker),
                "{CHANGELOG} is missing `{marker}`",
            );
        }
    }
}
