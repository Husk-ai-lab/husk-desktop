#!/usr/bin/env python3
"""Steps of the desktop-shell workflow (.github/workflows/release.yml), kept in one script so CI
and a laptop run the same logic. Standard library only; works on Python 3.10+.

    check-version   the three version files agree, and a tag is v<version>[-suffix]
    collect         name + hash the installers one build produced, check them, write its status file
    report          merge the per-target status files into the release report (and exit 1 when a
                    required target failed or is missing)

The report is always written first; only then does `report` fail. See README "Releasing".
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import platform
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCHEMA = "husk-desktop-release-report/1"
PRODUCT = "Husk-AI"
MIN_BUNDLE_BYTES = 1_000_000  # an installer smaller than this is a failed or empty build

# bundle sub-folder (under src-tauri/target/release/bundle) -> installer extensions
BUNDLE_EXTENSIONS = {
    "dmg": (".dmg",),
    "nsis": (".exe",),
    "msi": (".msi",),
    "appimage": (".AppImage",),
    "deb": (".deb",),
}

# What is true of every build today. `report` adds the ones that depend on the run.
KNOWN_GAPS = [
    "Installers are not notarised and Windows installers are not Authenticode-signed unless the "
    "signing secrets were present (see each target's signed/notarised flags).",
    "Authenticity is not proven: the shell checks the SHA-256 GitHub publishes for each backend "
    "asset (integrity), but there is no signed manifest, so someone who can edit a backend release "
    "can change asset and digest together.",
    "Shell self-update is not wired (tauri-plugin-updater is not enabled).",
    "The first-run test drives the shell's install/verify/supervise core against the real backend "
    "release on each runner; it does not launch the packaged GUI app, and the installers have not "
    "been exercised on real hardware.",
    "macOS arm64 and Intel are separate DMGs (no universal bundle yet).",
]


class GateError(Exception):
    """The version gate failed; the message is printed as is."""


# ------------------------------------------------------------------ check-version


def cargo_version(text: str) -> str:
    """`version` of the [package] table (no TOML library needed on 3.10)."""
    in_package = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_package = stripped == "[package]"
            continue
        if in_package:
            m = re.match(r'version\s*=\s*"([^"]+)"', stripped)
            if m:
                return m.group(1)
    raise GateError("no [package] version found in src-tauri/Cargo.toml")


def read_versions(root: Path) -> dict[str, str]:
    conf = json.loads((root / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8"))
    pkg = json.loads((root / "package.json").read_text(encoding="utf-8"))
    cargo = cargo_version((root / "src-tauri" / "Cargo.toml").read_text(encoding="utf-8"))
    return {
        "src-tauri/tauri.conf.json": str(conf.get("version", "")),
        "src-tauri/Cargo.toml": cargo,
        "package.json": str(pkg.get("version", "")),
    }


def check_version(root: Path, ref_type: str = "", ref_name: str = "") -> dict[str, str]:
    """Returns {"version", "label", "prerelease", "tag"}; raises GateError on any mismatch."""
    versions = read_versions(root)
    if len(set(versions.values())) != 1 or not next(iter(versions.values())):
        listing = ", ".join(f"{k}={v or '(none)'}" for k, v in versions.items())
        raise GateError(f"the three versions must be the same non-empty value: {listing}")
    version = next(iter(versions.values()))
    out = {"version": version, "label": version, "prerelease": "false", "tag": ""}
    if ref_type == "tag":
        m = re.fullmatch(r"v" + re.escape(version) + r"(-[0-9A-Za-z.-]+)?", ref_name)
        if not m:
            raise GateError(
                f"tag {ref_name!r} does not match the app version: expected v{version} or "
                f"v{version}-<suffix> (versions are {version} in all three files)"
            )
        out.update(label=ref_name[1:], prerelease="true" if m.group(1) else "false", tag=ref_name)
    return out


# ------------------------------------------------------------------ collect


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def find_bundles(bundle_dir: Path) -> list[Path]:
    found: list[Path] = []
    for sub, exts in BUNDLE_EXTENSIONS.items():
        d = bundle_dir / sub
        if d.is_dir():
            found += sorted(p for p in d.iterdir() if p.is_file() and p.suffix in exts)
    return found


def bundle_name(label: str, triple: str, src: Path) -> str:
    # no spaces (they end up in URLs), and the triple tells the two Mac DMGs apart
    return f"{PRODUCT}_{label}_{triple}{src.suffix}"


def check_macos_dmg(dmg: Path, version: str) -> str | None:
    """None when the DMG verifies and holds an app of this version; otherwise the reason."""
    r = subprocess.run(["hdiutil", "verify", str(dmg)], capture_output=True, text=True)
    if r.returncode != 0:
        return f"hdiutil verify failed: {r.stderr.strip() or r.stdout.strip()}"
    mount = tempfile.mkdtemp(prefix="husk-dmg-")
    attached = subprocess.run(
        ["hdiutil", "attach", "-readonly", "-nobrowse", "-noautoopen", "-mountpoint", mount, str(dmg)],
        capture_output=True,
        text=True,
    )
    if attached.returncode != 0:
        return f"hdiutil attach failed: {attached.stderr.strip()}"
    try:
        apps = sorted(Path(mount).glob("*.app"))
        if not apps:
            return "the DMG holds no .app"
        with (apps[0] / "Contents" / "Info.plist").open("rb") as f:
            info = plistlib.load(f)
        got = info.get("CFBundleShortVersionString")
        if got != version:
            return f"app version is {got!r}, expected {version!r}"
        return None
    finally:
        subprocess.run(["hdiutil", "detach", mount, "-force"], capture_output=True)
        shutil.rmtree(mount, ignore_errors=True)


def collect(
    *,
    triple: str,
    version: str,
    label: str,
    bundle_dir: Path,
    out: Path,
    e2e_file: Path | None,
    job_status: str,
    signed: bool,
    notarised: bool,
) -> dict:
    out.mkdir(parents=True, exist_ok=True)
    problems: list[str] = []
    bundles = []
    for src in find_bundles(bundle_dir):
        name = bundle_name(label, triple, src)
        dest = out / name
        shutil.copy2(src, dest)
        size = dest.stat().st_size
        if size < MIN_BUNDLE_BYTES:
            problems.append(f"{name} is only {size} bytes")
        if src.suffix == ".dmg" and platform.system() == "Darwin":
            reason = check_macos_dmg(dest, version)
            if reason:
                problems.append(f"{name}: {reason}")
        bundles.append({"name": name, "size": size, "sha256": sha256_of(dest)})
    if not bundles:
        problems.append(f"no installer found under {bundle_dir}")

    e2e = {"status": "not_run", "reason": "the first-run test did not write a result"}
    if e2e_file and e2e_file.is_file():
        try:
            e2e = json.loads(e2e_file.read_text(encoding="utf-8"))
        except ValueError as e:
            e2e = {"status": "failed", "reason": f"unreadable e2e result: {e}"}
    if e2e.get("status") == "failed":
        problems.append(f"first-run test failed: {e2e.get('reason', '')}")

    ok = job_status == "success" and not problems
    status = {
        "triple": triple,
        "status": "success" if ok else "failed",
        "job_status": job_status,
        "problems": problems,
        "bundles": bundles,
        "signed": signed,
        "notarised": notarised,
        "e2e": e2e,
    }
    (out / f"status-{triple}.json").write_text(json.dumps(status, indent=2) + "\n", encoding="utf-8")
    return status


# ------------------------------------------------------------------ report


def load_statuses(status_dir: Path) -> dict[str, dict]:
    found = {}
    for p in sorted(status_dir.rglob("status-*.json")):
        s = json.loads(p.read_text(encoding="utf-8"))
        found[s["triple"]] = s
    return found


def build_report(statuses: dict[str, dict], required: list[str], meta: dict) -> dict:
    targets = []
    ok = True
    for triple in sorted(set(statuses) | set(required)):
        s = statuses.get(triple) or {
            "triple": triple,
            "status": "missing",
            "problems": ["no status file: the build job did not finish"],
            "bundles": [],
            "signed": False,
            "notarised": False,
            "e2e": {"status": "not_run", "reason": "the build did not finish"},
        }
        is_required = triple in required
        s = dict(s, required=is_required)
        if is_required and s["status"] != "success":
            ok = False
        if is_required and s["e2e"].get("status") == "not_run":
            s["problems"] = list(s.get("problems", [])) + ["required target: the first-run test did not run"]
            ok = False
        targets.append(s)

    gaps = list(KNOWN_GAPS)
    for t in targets:
        if t["e2e"].get("status") == "skipped":
            gaps.append(f"{t['triple']}: first-run test skipped ({t['e2e'].get('reason', '')}).")
    return {
        "schema": SCHEMA,
        "component": "husk-desktop",
        "version": meta["version"],
        "label": meta["label"],
        "tag": meta["tag"],
        "commit": meta["commit"],
        "built_at": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "builder": meta["builder"],
        "required": required,
        "ok": ok,
        "targets": targets,
        "known_gaps": gaps,
    }


def _mb(n: int) -> str:
    return f"{n / 1_048_576:.1f} MB"


def render_markdown(r: dict) -> str:
    lines = [
        f"# Husk AI desktop shell {r['label']}",
        "",
        f"- Commit: `{r['commit'][:12] or 'unknown'}`  ·  built {r['built_at']}  ·  {r['builder']}",
        f"- Result: **{'OK' if r['ok'] else 'FAILED'}** (required: {', '.join(r['required']) or 'none'})",
        "",
        "| Target | Status | Required | First-run test | Signed | Notarised |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for t in r["targets"]:
        e2e = t["e2e"].get("status", "not_run")
        if t["e2e"].get("health_seconds") is not None:
            e2e += f" ({t['e2e']['health_seconds']:.1f} s to /health)"
        lines.append(
            f"| `{t['triple']}` | {t['status']} | {'yes' if t['required'] else 'no'} | {e2e} | "
            f"{'yes' if t['signed'] else 'no'} | {'yes' if t['notarised'] else 'no'} |"
        )
    lines += ["", "## Installers", "", "| File | Size | SHA-256 |", "| --- | --- | --- |"]
    for t in r["targets"]:
        for b in t["bundles"]:
            lines.append(f"| `{b['name']}` | {_mb(b['size'])} | `{b['sha256']}` |")
    problems = [(t["triple"], p) for t in r["targets"] for p in t.get("problems", [])]
    if problems:
        lines += ["", "## Problems", ""] + [f"- `{triple}`: {p}" for triple, p in problems]
    lines += ["", "## Known gaps", ""] + [f"- {g}" for g in r["known_gaps"]]
    return "\n".join(lines) + "\n"


def write_report(status_dir: Path, out: Path, required: list[str], meta: dict) -> dict:
    out.mkdir(parents=True, exist_ok=True)
    report = build_report(load_statuses(status_dir), required, meta)
    (out / "shell-release-report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    (out / "shell-release-report.md").write_text(render_markdown(report), encoding="utf-8")
    sums = [f"{b['sha256']}  {b['name']}" for t in report["targets"] for b in t["bundles"]]
    (out / "SHA256SUMS").write_text("\n".join(sums) + ("\n" if sums else ""), encoding="utf-8")
    return report


# ------------------------------------------------------------------ command line


def _flag(value: str) -> bool:
    return value.strip().lower() in ("1", "true", "yes")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("check-version")
    p.add_argument("--root", type=Path, default=ROOT)
    p.add_argument("--ref-type", default="")
    p.add_argument("--ref-name", default="")

    p = sub.add_parser("collect")
    p.add_argument("--triple", required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--label", required=True)
    p.add_argument("--bundle-dir", type=Path, default=ROOT / "src-tauri" / "target" / "release" / "bundle")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--e2e-result", type=Path)
    p.add_argument("--job-status", default="success")
    p.add_argument("--signed", default="false")
    p.add_argument("--notarised", default="false")

    p = sub.add_parser("report")
    p.add_argument("--status-dir", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--require", default="")
    p.add_argument("--version", required=True)
    p.add_argument("--label", required=True)
    p.add_argument("--tag", default="")
    p.add_argument("--commit", default="")
    p.add_argument("--builder", default="local")

    a = ap.parse_args(argv)

    if a.cmd == "check-version":
        try:
            res = check_version(a.root, a.ref_type, a.ref_name)
        except GateError as e:
            print(f"version gate failed: {e}", file=sys.stderr)
            return 1
        for k in ("version", "label", "prerelease", "tag"):
            print(f"{k}={res[k]}")
        return 0

    if a.cmd == "collect":
        s = collect(
            triple=a.triple,
            version=a.version,
            label=a.label,
            bundle_dir=a.bundle_dir,
            out=a.out,
            e2e_file=a.e2e_result,
            job_status=a.job_status,
            signed=_flag(a.signed),
            notarised=_flag(a.notarised),
        )
        print(json.dumps(s, indent=2))
        return 0 if s["status"] == "success" else 1

    required = [t.strip() for t in a.require.split(",") if t.strip()]
    meta = {
        "version": a.version,
        "label": a.label,
        "tag": a.tag,
        "commit": a.commit or os.environ.get("GITHUB_SHA", ""),
        "builder": a.builder,
    }
    report = write_report(a.status_dir, a.out, required, meta)
    print(render_markdown(report))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
