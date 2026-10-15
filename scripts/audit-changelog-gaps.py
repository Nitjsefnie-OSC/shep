#!/usr/bin/env python3
"""Report released changelog sections that dropped a commit's entry.

`scripts/fix-release-bump.py` rewrites a release pull request before it
merges. This audits what already merged. For every crate with a
`CHANGELOG.md` and every released version, it reads the commits between the
previous release tag and this one that touched the crate's directory, keeps
the ones `release-plz-changelog.toml` would have given a changelog line
(`feat`, `fix`, `perf`, a non-skipped `refactor`, anything breaking), and
reports the lines that version's section does not carry.

Tags. The released versions come from the tags the config's `tag_pattern`
(`v[0-9].*`) matches, read the way git-cliff reads it: an unanchored search,
so every per-package tag matches. Because the workspace releases one shared
version, a tag on any crate dates that version for every crate, and the
commit carrying the most tags at a version is the release commit the ranges
are cut at. The `shep-v0.9.0` tag is therefore not stray here: it is the
`shep` package's own per-package tag (the CLI crate in `crates/shep-cli/`)
and it dates 0.9.0 like any other. The hand-pushed bare `v0.1.0` and
`v0.1.0-alpha.1` tags match the pattern the same way and date their versions;
`pre-rebase-backup` matches nothing in the pattern and is ignored. A
pre-release number carries no changelog section, so it counts as a range
boundary but is never itself audited.

Ranges use `git log --full-history` through the shared guard code, so a
commit on a diamond leg the release-plz walk never crossed is still seen. A
parentless root commit has no first parent for the guard's diff, so it is
diffed against the empty tree instead, the same bound the first range
starts at.

Output, per crate: released sections, empty sections, empty sections that
owed an entry, and missing lines in total. Then one line per gap: crate,
version, commit, subject. It reports and writes nothing, and exits 0.

Usage:

    python3 scripts/audit-changelog-gaps.py
"""

from __future__ import annotations

import argparse
import importlib.util
import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent

# scripts/fix-release-bump.py has a hyphen in its name, so it loads the way
# scripts/test_fix_release_bump.py loads it: by file location, under a module
# name the audit can spell.
_spec = importlib.util.spec_from_file_location("fix_release_bump", SCRIPTS / "fix-release-bump.py")
fix = importlib.util.module_from_spec(_spec)
sys.modules["fix_release_bump"] = fix
_spec.loader.exec_module(fix)

# git's empty tree, the lower bound of the first release's range.
EMPTY_TREE = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"

# `shep-core-v0.9.0`, `shep-v0.9.0`, or the hand-pushed bare `v0.1.0`.
TAG_NAME = re.compile(r"^(?:(?P<crate>.+)-)?v(?P<version>[^ ]+)$")


@dataclass(frozen=True)
class Crate:
    name: str
    directory: str
    changelog: str


@dataclass(frozen=True)
class Gap:
    crate: str
    version: str
    sha: str
    subject: str


@dataclass
class CrateReport:
    crate: Crate
    sections: int = 0
    empty: int = 0
    owed: int = 0
    missing: int = 0
    gaps: list[Gap] = field(default_factory=list)


def tag_pattern(repo: fix.Repo) -> str:
    """The `tag_pattern` release-plz-changelog.toml tells git-cliff to use."""
    config = tomllib.loads((repo.path / "release-plz-changelog.toml").read_text())
    return config.get("git", {}).get("tag_pattern", "v[0-9].*")


def crates_with_changelogs(repo: fix.Repo) -> list[Crate]:
    """Every crate directory under crates/ that carries a CHANGELOG.md.

    The package name comes from the manifest, because that is the prefix
    release-plz writes into the crate's tags.
    """
    crates: list[Crate] = []
    for changelog in sorted((repo.path / "crates").glob("*/CHANGELOG.md")):
        directory = f"crates/{changelog.parent.name}"
        manifest = changelog.parent / "Cargo.toml"
        name = tomllib.loads(manifest.read_text())["package"]["name"]
        crates.append(Crate(name=name, directory=directory, changelog=f"{directory}/CHANGELOG.md"))
    return crates


def pre_release_key(pre: str) -> tuple[tuple[int, int, str | int], ...]:
    """Semver order for a pre-release: identifiers compare one by one, a
    numeric one as a number and below an alphanumeric one, and a longer set
    above an equal shorter one. `alpha.10` sits over `alpha.2`, which string
    order gets wrong. Build metadata is already cut off by the caller and
    does not rank."""
    if not pre:
        return ()
    return tuple(
        (0, int(part), len(part)) if part.isdigit() else (1, 0, part)
        for part in pre.split(".")
    )


def tag_sort_key(version: str) -> tuple[int, int, int, int, tuple[tuple[int, int, str | int], ...]]:
    """Semver order, with a pre-release sorting below its plain version.

    `0.1.0-alpha.1` sits under `0.1.0`, which is what makes it a range
    boundary for it rather than a gap in the walk.
    """
    number, _, pre = version.partition("-")
    parts = number.split("+")[0].split(".")
    major, minor, patch = (int(part) if part.isdigit() else 0 for part in parts[:3])
    pre = pre.split("+")[0]
    return (major, minor, patch, 0 if pre else 1, pre_release_key(pre))


def release_timeline(repo: fix.Repo) -> list[tuple[str, tuple[int, int, int] | None, str]]:
    """(tag version, audited version or None, release commit), oldest first.

    A version tagged by several crates takes the commit those tags crowd
    onto, which is the release commit; a tie breaks on the tag name so the
    order never depends on git's. A pre-release number carries no changelog
    section, so its version is None: it bounds the next range but is never
    itself audited.
    """
    pattern = re.compile(tag_pattern(repo))
    rows = repo.git(
        "for-each-ref",
        "refs/tags",
        "--format=%(refname:short)\t%(*objectname:short)\t%(objectname:short)",
    )
    # tag version -> commit -> (how many matching tags point at it, first tag name)
    candidates: dict[str, dict[str, tuple[int, str]]] = {}
    for row in rows.splitlines():
        name, peeled, object_name = row.split("\t")
        if not pattern.search(name):
            continue
        match = TAG_NAME.match(name)
        if match is None:
            continue
        commit = peeled or object_name
        at = candidates.setdefault(match.group("version"), {})
        count, first = at.get(commit, (0, name))
        at[commit] = (count + 1, min(first, name))
    timeline = [
        (
            version,
            fix.parse_version(version),
            max(by_commit.items(), key=lambda item: (item[1][0], item[1][1], item[0]))[0],
        )
        for version, by_commit in candidates.items()
    ]
    timeline.sort(key=lambda entry: tag_sort_key(entry[0]))
    return timeline


def versioned_sections(text: str) -> dict[tuple[int, int, int], list[str]]:
    """{version: body lines} for every `## [x.y.z]` section of a changelog."""
    _head, sections = fix.split_sections(text)
    found: dict[tuple[int, int, int], list[str]] = {}
    for heading, body in sections:
        version = fix.section_version(heading)
        if version is not None:
            found[version] = body
    return found


def touched_paths(repo: fix.Repo, sha: str) -> list[str]:
    """Files the commit brought into the tree, the guard's way, plus the one
    commit the guard cannot read: a parentless root commit has no `^1`, so
    the guard's diff fails and it reports nothing. Diff it against the empty
    tree, the bound the first release's range starts at, or the commit that
    created a crate is never matched to that crate.
    """
    paths = fix.touched_paths(repo, sha)
    if paths:
        return paths
    if repo.try_git("rev-parse", "--quiet", "--verify", f"{sha}^1") is not None:
        return paths
    raw = repo.try_git("diff", "--name-only", EMPTY_TREE, sha)
    return [line for line in (raw or "").splitlines() if line]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", default=str(SCRIPTS.parent))
    args = parser.parse_args(argv)

    repo = fix.Repo(Path(args.repo))
    rules = fix.load_changelog_rules(repo)
    crates = crates_with_changelogs(repo)
    timeline = release_timeline(repo)

    # One git read per commit, shared by every crate and version: the range
    # each version transition covers, the changelog line each commit owes,
    # and the files each surviving commit brought into the tree.
    ranges: list[list[fix.Commit]] = []
    for index, (_version, _audited, commit) in enumerate(timeline):
        since = timeline[index - 1][2] if index else EMPTY_TREE
        ranges.append(fix.collect_commits(repo, since, commit) if since != commit else [])

    entries = {
        commit.sha: fix.entry_for(commit, rules)
        for commits in ranges
        for commit in commits
    }
    subjects = {
        commit.sha: commit.subject
        for commits in ranges
        for commit in commits
    }
    paths = {
        sha: touched_paths(repo, sha)
        for sha, entry in entries.items()
        if entry is not None
    }

    reports: list[CrateReport] = []
    for crate in crates:
        report = CrateReport(crate=crate)
        text = (repo.path / crate.changelog).read_text()
        sections = versioned_sections(text)
        report.sections = len(sections)
        report.empty = sum(
            1 for body in sections.values() if not fix.existing_entry_lines(body)
        )
        for index, (_tag_version, version, _commit) in enumerate(timeline):
            if version is None:
                continue
            body = sections.get(version)
            if body is None:
                continue
            prefix = f"{crate.directory}/"
            owed: list[fix.Entry] = []
            for commit in ranges[index]:
                entry = entries[commit.sha]
                if entry is None:
                    continue
                if any(path.startswith(prefix) for path in paths[commit.sha]):
                    owed.append(entry)
            missing = fix.missing_entries(owed, fix.existing_entry_lines(body))
            report.missing += len(missing)
            if not fix.existing_entry_lines(body) and owed:
                report.owed += 1
            for entry in missing:
                report.gaps.append(
                    Gap(
                        crate=crate.name,
                        version=fix.show_version(version),
                        sha=entry.sha[:10],
                        subject=subjects[entry.sha],
                    )
                )
        reports.append(report)

    for report in reports:
        print(
            f"{report.crate.name}: sections={report.sections}, "
            f"empty={report.empty}, owed={report.owed}, missing={report.missing}"
        )
    gaps = [gap for report in reports for gap in report.gaps]
    if not gaps:
        print("no gaps")
    for gap in gaps:
        print(f"gap: {gap.crate} {gap.version} {gap.sha} {gap.subject}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
