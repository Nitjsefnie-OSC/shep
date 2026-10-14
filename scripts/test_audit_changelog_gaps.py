"""Tests for scripts/audit-changelog-gaps.py.

Run them with:

    python3 -m unittest discover --start-directory scripts

The fixture builds a throwaway two-crate repository whose history is five
releases, each tagged per crate the way release-plz tags one, with one
release per shape the audit has to tell apart: a feat whose section came out
empty, a chore-only release, a docs commit, a breaking chore, a commit that
only touched the other crate, and a section that already carries its line.
Nothing here touches the real repository.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
REPO_ROOT = SCRIPTS.parent

HEADER = (
    "# Changelog\n"
    "\n"
    "All notable changes to this crate are documented in this file.\n"
    "\n"
    "The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),\n"
    "and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).\n"
    "\n"
)


class Fixture(unittest.TestCase):
    """A two-crate repository with five tagged releases, one per audit case."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.repo = self.tmp / "repo"
        self.repo.mkdir()

        self.git("init", "-q", "--initial-branch=main")
        self.git("config", "user.name", "fixture")
        self.git("config", "user.email", "fixture@example.invalid")

        for name in ("release-plz.toml", "release-plz-changelog.toml"):
            (self.repo / name).write_text((REPO_ROOT / name).read_text())

        self._write_manifests()
        for crate in ("shep-core", "shep-daemon"):
            self.write(f"crates/{crate}/CHANGELOG.md", HEADER + "## [Unreleased]\n\n")

        # 0.1.0: a feat whose line release-plz did write. The baseline that
        # the audit must not report.
        self.write("crates/shep-core/src/lib.rs", "pub fn base() {}\n")
        self.commit("feat(core): base work", date="2026-01-01T00:00:00")
        self._release(
            "0.1.0",
            {"shep-core": "### Added\n\n- Base work\n"},
            date="2026-01-02",
        )

        # 0.2.0: a feat whose section came out empty, a docs commit that
        # owes no line, and a feat that only reached the other crate.
        self.write("crates/shep-core/src/gap.rs", "pub fn carry() {}\n")
        self.commit("feat(core): carry a gap", date="2026-01-03T00:00:00")
        self.write("crates/shep-core/docs/note.md", "a note\n")
        self.commit("docs(core): a note", date="2026-01-03T01:00:00")
        self.write("crates/shep-daemon/src/lib.rs", "pub fn watch() {}\n")
        self.commit("feat(daemon): watch the fold", date="2026-01-03T02:00:00")
        self._release("0.2.0", {}, date="2026-01-04")

        # 0.3.0: a chore-only release. Its empty section is expected.
        self.write("crates/shep-core/src/lib.rs", "pub fn base() {}\n// tidy\n")
        self.commit("chore: tidy a comment", date="2026-01-05T00:00:00")
        self._release("0.3.0", {}, date="2026-01-06")

        # 0.4.0: a breaking chore. protect_breaking_commits keeps its line.
        self.write("crates/shep-core/src/lib.rs", "pub fn base() {}\n")
        self.commit("chore(core)!: drop a legacy flag", date="2026-01-07T00:00:00")
        self._release("0.4.0", {}, date="2026-01-08")

        # 0.5.0: a fix whose line the section already carries.
        self.write("crates/shep-core/src/gap.rs", "pub fn carry() {}\n// closed\n")
        self.commit("fix(core): close the gap", date="2026-01-09T00:00:00")
        self._release(
            "0.5.0",
            {"shep-core": "### Fixed\n\n- Close the gap\n"},
            date="2026-01-10",
        )

    def tearDown(self) -> None:
        self._tmp.cleanup()

    # Building it.

    def git(self, *args: str, date: str | None = None) -> str:
        env = dict(os.environ)
        if date:
            env["GIT_AUTHOR_DATE"] = date
            env["GIT_COMMITTER_DATE"] = date
        return subprocess.run(
            ["git", *args],
            cwd=self.repo,
            check=True,
            capture_output=True,
            text=True,
            env=env,
        ).stdout

    def write(self, path: str, text: str) -> None:
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def commit(self, subject: str, date: str) -> None:
        self.git("add", "-A")
        self.git("commit", "-q", "-m", subject, date=date)

    def sha_of(self, subject: str) -> str:
        return self.git("log", "-1", "--format=%H", f"--grep={subject}").strip()

    def _write_manifests(self) -> None:
        self.write(
            "Cargo.toml",
            '[workspace]\n'
            'resolver = "3"\n'
            'members = [\n'
            '    "crates/shep-core",\n'
            '    "crates/shep-daemon",\n'
            ']\n'
            '\n'
            '[workspace.package]\n'
            'version = "0.1.0"\n'
            'edition = "2024"\n',
        )
        for directory, name in (("crates/shep-core", "shep-core"), ("crates/shep-daemon", "shep-daemon")):
            self.write(
                f"{directory}/Cargo.toml",
                "[package]\n"
                f'name = "{name}"\n'
                'description = "fixture"\n'
                "version.workspace = true\n"
                "edition.workspace = true\n",
            )

    def _release(self, version: str, sections: dict[str, str], date: str) -> None:
        """Append this release's section to each changelog, commit it, and tag
        every crate at it the way release-plz's per-package tags do."""
        for crate in ("shep-core", "shep-daemon"):
            path = f"crates/{crate}/CHANGELOG.md"
            text = (self.repo / path).read_text()
            body = sections.get(crate, "")
            section = f"## [{version}] - {date}\n\n{body}\n" if body else f"## [{version}] - {date}\n\n\n"
            self.write(path, text.replace("## [Unreleased]\n\n", "## [Unreleased]\n\n" + section))
        self.commit(f"chore: release v{version}", date=f"{date}T00:00:00")
        for crate in ("shep-core", "shep-daemon"):
            self.git("tag", f"{crate}-v{version}")

    # Running the audit.

    def run_audit(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPTS / "audit-changelog-gaps.py"), "--repo", str(self.repo)],
            check=False,
            capture_output=True,
            text=True,
        )

    def counts(self, stdout: str, crate: str) -> dict[str, int]:
        for line in stdout.splitlines():
            if line.startswith(f"{crate}: "):
                pairs = line.split(": ", 1)[1].split(", ")
                return {key: int(value) for key, value in (p.split("=") for p in pairs)}
        raise AssertionError(f"no count line for {crate} in:\n{stdout}")

    def gaps(self, stdout: str) -> list[str]:
        return [line for line in stdout.splitlines() if line.startswith("gap: ")]

    def gap_lines(self, stdout: str, crate: str, version: str) -> list[str]:
        marker = f"gap: {crate} {version} "
        return [line for line in self.gaps(stdout) if line.startswith(marker)]


class TestAudit(Fixture):
    def test_a_feat_with_an_empty_section_is_reported(self) -> None:
        run = self.run_audit()
        self.assertEqual(run.returncode, 0, run.stderr)
        core = self.counts(run.stdout, "shep-core")
        self.assertEqual(
            core,
            {"sections": 5, "empty": 3, "owed": 2, "missing": 2},
        )
        gaps = self.gap_lines(run.stdout, "shep-core", "0.2.0")
        self.assertEqual(len(gaps), 1)
        self.assertIn("feat(core): carry a gap", gaps[0])
        self.assertIn(self.sha_of("feat(core): carry a gap")[:10], gaps[0])

    def test_a_chore_only_release_is_expected_empty(self) -> None:
        run = self.run_audit()
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(self.gap_lines(run.stdout, "shep-core", "0.3.0"), [])
        core = self.counts(run.stdout, "shep-core")
        self.assertEqual(core["empty"], 3)
        self.assertEqual(core["owed"], 2)

    def test_a_docs_commit_is_not_owed_a_line(self) -> None:
        run = self.run_audit()
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertNotIn("docs(core): a note", run.stdout)
        # 0.2.0 is owed, but only for the feat that reached the crate.
        self.assertEqual(len(self.gap_lines(run.stdout, "shep-core", "0.2.0")), 1)

    def test_a_breaking_chore_is_owed_a_line(self) -> None:
        run = self.run_audit()
        self.assertEqual(run.returncode, 0, run.stderr)
        gaps = self.gap_lines(run.stdout, "shep-core", "0.4.0")
        self.assertEqual(len(gaps), 1)
        self.assertIn("chore(core)!: drop a legacy flag", gaps[0])
        core = self.counts(run.stdout, "shep-core")
        self.assertEqual(core["owed"], 2)
        self.assertEqual(core["missing"], 2)

    def test_a_commit_touching_another_crate_is_not_counted(self) -> None:
        run = self.run_audit()
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertNotIn(
            "feat(daemon): watch the fold",
            "\n".join(self.gap_lines(run.stdout, "shep-core", "0.2.0")),
        )
        daemon_gaps = self.gap_lines(run.stdout, "shep-daemon", "0.2.0")
        self.assertEqual(len(daemon_gaps), 1)
        self.assertIn("feat(daemon): watch the fold", daemon_gaps[0])
        daemon = self.counts(run.stdout, "shep-daemon")
        self.assertEqual(
            daemon,
            {"sections": 5, "empty": 5, "owed": 1, "missing": 1},
        )

    def test_a_section_that_has_its_line_is_not_reported(self) -> None:
        run = self.run_audit()
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(self.gap_lines(run.stdout, "shep-core", "0.5.0"), [])
        self.assertEqual(self.gap_lines(run.stdout, "shep-core", "0.1.0"), [])
        self.assertNotIn("fix(core): close the gap", run.stdout)


if __name__ == "__main__":
    unittest.main()
