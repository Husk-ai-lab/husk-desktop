"""Tests for scripts/release.py. Run: python3 -m unittest scripts/test_release.py"""

import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release  # noqa: E402


def make_repo(root: Path, conf="0.1.0", cargo="0.1.0", pkg="0.1.0") -> Path:
    (root / "src-tauri").mkdir(parents=True)
    (root / "src-tauri" / "tauri.conf.json").write_text(json.dumps({"version": conf}))
    (root / "src-tauri" / "Cargo.toml").write_text(
        f'[package]\nname = "husk-desktop"\nversion = "{cargo}"\nedition = "2021"\n\n'
        '[dependencies]\nserde = { version = "1" }\n'
    )
    (root / "package.json").write_text(json.dumps({"version": pkg}))
    return root


class VersionGate(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)

    def test_equal_versions_pass_on_a_branch(self):
        r = release.check_version(make_repo(self.tmp / "a"))
        self.assertEqual(r["version"], "0.1.0")
        self.assertEqual(r["prerelease"], "false")

    def test_dependency_version_is_not_mistaken_for_the_package_version(self):
        self.assertEqual(release.cargo_version('[dependencies]\nx = { version = "9" }\n[package]\nversion = "1.2.3"\n'), "1.2.3")

    def test_one_file_differing_fails_and_names_all_three(self):
        root = make_repo(self.tmp / "b", pkg="0.2.0")
        with self.assertRaises(release.GateError) as cm:
            release.check_version(root)
        msg = str(cm.exception)
        for f in ("tauri.conf.json=0.1.0", "Cargo.toml=0.1.0", "package.json=0.2.0"):
            self.assertIn(f, msg)

    def test_tag_must_match_the_app_version(self):
        root = make_repo(self.tmp / "c")
        with self.assertRaises(release.GateError):
            release.check_version(root, "tag", "v0.2.0")
        with self.assertRaises(release.GateError):
            release.check_version(root, "tag", "v0.1.0.1")
        with self.assertRaises(release.GateError):
            release.check_version(root, "tag", "v0.1.0; rm -rf /")

    def test_exact_tag_is_stable_and_suffixed_tag_is_a_prerelease(self):
        root = make_repo(self.tmp / "d")
        stable = release.check_version(root, "tag", "v0.1.0")
        self.assertEqual((stable["label"], stable["prerelease"]), ("0.1.0", "false"))
        rc = release.check_version(root, "tag", "v0.1.0-rc.1")
        self.assertEqual((rc["label"], rc["prerelease"], rc["tag"]), ("0.1.0-rc.1", "true", "v0.1.0-rc.1"))

    def test_a_branch_ref_name_is_not_judged(self):
        root = make_repo(self.tmp / "e")
        self.assertEqual(release.check_version(root, "branch", "main")["tag"], "")

    def test_command_line_exit_codes_and_output(self):
        good = make_repo(self.tmp / "f")
        bad = make_repo(self.tmp / "g", cargo="0.3.0")
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(release.main(["check-version", "--root", str(good)]), 0)
        self.assertIn("version=0.1.0", out.getvalue())
        err = io.StringIO()
        with redirect_stderr(err):
            self.assertEqual(release.main(["check-version", "--root", str(bad)]), 1)
        self.assertIn("version gate failed", err.getvalue())

    def test_the_real_repository_passes(self):
        # the shipped files must always satisfy their own gate
        release.check_version(release.ROOT)


class Collect(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)
        self.bundle = self.tmp / "bundle"
        (self.bundle / "nsis").mkdir(parents=True)
        (self.bundle / "nsis" / "Husk AI_0.1.0_x64-setup.exe").write_bytes(b"x" * 1_500_000)
        (self.bundle / "nsis" / "notes.txt").write_text("ignored")
        self.out = self.tmp / "out"

    def run_collect(self, **kw):
        args = dict(
            triple="x86_64-pc-windows-msvc",
            version="0.1.0",
            label="0.1.0",
            bundle_dir=self.bundle,
            out=self.out,
            e2e_file=None,
            job_status="success",
            signed=False,
            notarised=False,
        )
        args.update(kw)
        return release.collect(**args)

    def test_installer_is_renamed_hashed_and_status_written(self):
        e2e = self.tmp / "e2e.json"
        e2e.write_text(json.dumps({"status": "passed", "health_seconds": 3.2}))
        s = self.run_collect(e2e_file=e2e)
        self.assertEqual(s["status"], "success")
        self.assertEqual([b["name"] for b in s["bundles"]], ["Husk-AI_0.1.0_x86_64-pc-windows-msvc.exe"])
        self.assertEqual(s["bundles"][0]["size"], 1_500_000)
        self.assertEqual(len(s["bundles"][0]["sha256"]), 64)
        self.assertTrue((self.out / "status-x86_64-pc-windows-msvc.json").is_file())

    def test_no_installer_is_a_failure(self):
        for p in (self.bundle / "nsis").iterdir():
            p.unlink()
        s = self.run_collect()
        self.assertEqual(s["status"], "failed")
        self.assertTrue(any("no installer" in p for p in s["problems"]))

    def test_tiny_installer_and_failed_job_and_failed_e2e_each_fail(self):
        (self.bundle / "nsis" / "Husk AI_0.1.0_x64-setup.exe").write_bytes(b"x")
        self.assertEqual(self.run_collect()["status"], "failed")
        (self.bundle / "nsis" / "Husk AI_0.1.0_x64-setup.exe").write_bytes(b"x" * 1_500_000)
        self.assertEqual(self.run_collect(job_status="failure")["status"], "failed")
        e2e = self.tmp / "e2e.json"
        e2e.write_text(json.dumps({"status": "failed", "reason": "health timeout"}))
        s = self.run_collect(e2e_file=e2e)
        self.assertEqual(s["status"], "failed")
        self.assertTrue(any("health timeout" in p for p in s["problems"]))


def status(triple, ok=True, e2e="passed", signed=False):
    return {
        "triple": triple,
        "status": "success" if ok else "failed",
        "problems": [] if ok else ["boom"],
        "bundles": [{"name": f"Husk-AI_0.1.0_{triple}.bin", "size": 2_000_000, "sha256": "ab" * 32}],
        "signed": signed,
        "notarised": signed,
        "e2e": {"status": e2e, "reason": "no asset for this triple" if e2e == "skipped" else ""},
    }


META = {"version": "0.1.0", "label": "0.1.0", "tag": "", "commit": "c" * 40, "builder": "test"}
A, X, W, L = "aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"


class Report(unittest.TestCase):
    def test_all_required_green_is_ok_and_lists_gaps(self):
        r = release.build_report({t: status(t) for t in (A, X, W)}, [A, X, W], META)
        self.assertTrue(r["ok"])
        self.assertEqual(r["component"], "husk-desktop")
        self.assertEqual(r["schema"], release.SCHEMA)
        self.assertTrue(any("Authenticity" in g for g in r["known_gaps"]))

    def test_a_failed_required_target_fails_the_report(self):
        r = release.build_report({A: status(A), X: status(X, ok=False), W: status(W)}, [A, X, W], META)
        self.assertFalse(r["ok"])

    def test_a_missing_required_target_fails_the_report_and_is_listed(self):
        r = release.build_report({A: status(A)}, [A, W], META)
        self.assertFalse(r["ok"])
        missing = next(t for t in r["targets"] if t["triple"] == W)
        self.assertEqual(missing["status"], "missing")

    def test_a_failed_experimental_target_does_not_gate(self):
        r = release.build_report({A: status(A), L: status(L, ok=False)}, [A], META)
        self.assertTrue(r["ok"])
        self.assertEqual(next(t for t in r["targets"] if t["triple"] == L)["required"], False)

    def test_skipped_e2e_passes_but_is_reported_as_a_gap(self):
        r = release.build_report({A: status(A, e2e="skipped")}, [A], META)
        self.assertTrue(r["ok"])
        self.assertTrue(any("skipped" in g for g in r["known_gaps"]))

    def test_required_target_whose_e2e_never_ran_fails(self):
        r = release.build_report({A: status(A, e2e="not_run")}, [A], META)
        self.assertFalse(r["ok"])

    def test_files_are_written_even_when_the_report_fails(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            (d / "in").mkdir()
            (d / "in" / f"status-{A}.json").write_text(json.dumps(status(A, ok=False)))
            with redirect_stdout(io.StringIO()):
                code = release.main(
                    ["report", "--status-dir", str(d / "in"), "--out", str(d / "out"), "--require", A, "--version", "0.1.0", "--label", "0.1.0"]
                )
            self.assertEqual(code, 1)
            self.assertTrue((d / "out" / "shell-release-report.json").is_file())
            self.assertTrue((d / "out" / "shell-release-report.md").is_file())
            self.assertIn("Husk-AI_0.1.0_", (d / "out" / "SHA256SUMS").read_text())


if __name__ == "__main__":
    unittest.main()
