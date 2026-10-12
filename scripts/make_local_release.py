#!/usr/bin/env python3
"""Package a PyInstaller one-folder build as a local, GitHub-shaped release for testing the shell.

    # real build (from the Husk repo: `uv run python installer.py`)
    python3 scripts/make_local_release.py --source ../HuskAI-core/dist/husk-backend --version 0.1.0

    # no build available: generate a tiny fake backend (macOS/Linux only)
    python3 scripts/make_local_release.py --fake --version 0.1.0
    python3 scripts/make_local_release.py --fake --component studio --version 0.1.0

    python3 scripts/serve_local_release.py            # then, in another terminal:
    HUSK_DESKTOP_GITHUB_API=http://127.0.0.1:8800/api npm run tauri dev

Output goes to .local-release/ (git-ignored):
    download/<prefix>-<version>-<triple>.(tar.gz|zip)   the asset, named as the plan requires
    api/<owner>/<repo>/latest.json, list.json           what GitHub's releases API would return
Nothing here signs or hashes anything: the shell does not verify integrity yet (plan rule 5 is
deferred), so neither does this script. The shell only checks the byte count.
"""
from __future__ import annotations

import argparse
import io
import hashlib
import json
import platform
import shutil
import stat
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / ".local-release"

COMPONENTS = {
    "husk": {"repo": "Husk-ai-lab/HuskAI-core", "prefix": "husk-backend", "exe": "husk-backend", "port": 8765},
    "studio": {"repo": "Husk-ai-lab/husk-ai", "prefix": "husk-studio", "exe": "husk-studio", "port": 7654},
}

FAKE_SERVER = r'''#!/bin/sh
# Fake {name} backend for testing the Husk AI desktop shell. Not a real product.
exec python3 -I -c "
import http.server, os, sys
name = {name!r}
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith('/health'):
            body, ctype = b'{{\"status\":\"ok\"}}', 'application/json'
        else:
            body = ('<!doctype html><meta charset=utf-8><title>Fake ' + name + '</title>'
                    '<body style=\"font:18px system-ui;padding:3rem\"><h1>Fake ' + name + ' backend</h1>'
                    '<p>Served by the test backend on port ' + os.environ.get('HUSK_PORT', '?') + '.</p>'
                    '<p>Data dir: <code>' + os.environ.get('HUSK_DATA_DIR', '?') + '</code></p>'
                    '<p><a href=\"https://example.com\">external link (should open the system browser)</a></p>'
                    '<p><a href=\"http://127.0.0.1:7654/runs/demo\">link to Studio (should open the Studio window)</a></p>'
                    '</body>').encode()
            ctype = 'text/html; charset=utf-8'
        self.send_response(200)
        self.send_header('Content-Type', ctype)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a):
        sys.stderr.write('request ' + self.path + '\\n')
http.server.ThreadingHTTPServer(('127.0.0.1', int(os.environ['HUSK_PORT'])), H).serve_forever()
"
'''


def host_triple() -> str:
    m, s = platform.machine().lower(), sys.platform
    arm = m in ("arm64", "aarch64")
    if s == "darwin":
        return "aarch64-apple-darwin" if arm else "x86_64-apple-darwin"
    if s.startswith("win"):
        return "x86_64-pc-windows-msvc"
    return "x86_64-unknown-linux-gnu"


def make_fake(dest: Path, exe: str, name: str) -> None:
    if sys.platform.startswith("win"):
        sys.exit("--fake needs a POSIX shell; on Windows pass --source with a real build")
    (dest / "_internal").mkdir(parents=True)
    (dest / "_internal" / "base_library.zip").write_bytes(b"fake")
    f = dest / exe
    f.write_text(FAKE_SERVER.format(name=name))
    f.chmod(f.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def archive(src: Path, out_file: Path, top: str) -> None:
    if out_file.name.endswith(".zip"):
        with zipfile.ZipFile(out_file, "w", zipfile.ZIP_DEFLATED) as z:
            for p in sorted(src.rglob("*")):
                z.write(p, f"{top}/{p.relative_to(src)}")
    else:
        with tarfile.open(out_file, "w:gz") as t:
            t.add(src, arcname=top)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--component", choices=COMPONENTS, default="husk")
    ap.add_argument("--version", required=True, help="e.g. 0.1.0 (the tag will be v0.1.0)")
    ap.add_argument("--source", type=Path, help="PyInstaller one-folder output (dist/husk-backend)")
    ap.add_argument("--fake", action="store_true", help="generate a tiny fake backend instead")
    ap.add_argument("--triple", default=host_triple())
    ap.add_argument("--base-url", default="http://127.0.0.1:8800", help="where serve_local_release.py listens")
    ap.add_argument("--prerelease", action="store_true")
    a = ap.parse_args()

    c = COMPONENTS[a.component]
    exe = c["exe"] + (".exe" if "windows" in a.triple else "")
    with tempfile.TemporaryDirectory() as tmp:
        staged = Path(tmp) / c["exe"]
        if a.fake:
            make_fake(staged, exe, c["exe"])
        elif a.source:
            if not (a.source / exe).is_file():
                sys.exit(f"{a.source / exe} not found: pass the PyInstaller one-folder output")
            shutil.copytree(a.source, staged, symlinks=True)
        else:
            sys.exit("pass --source <dist folder> or --fake")

        ext = "zip" if "windows" in a.triple else "tar.gz"
        name = f"{c['prefix']}-{a.version}-{a.triple}.{ext}"
        (OUT / "download").mkdir(parents=True, exist_ok=True)
        asset = OUT / "download" / name
        archive(staged, asset, c["exe"])

    release = {
        "tag_name": f"v{a.version}",
        "draft": False,
        "prerelease": a.prerelease,
        "assets": [{
            "name": name,
            "size": asset.stat().st_size,
            # GitHub publishes this too; the shell refuses an asset without it
            "digest": "sha256:" + hashlib.sha256(asset.read_bytes()).hexdigest(),
            "browser_download_url": f"{a.base_url}/download/{name}",
        }],
    }
    api = OUT / "api" / c["repo"]
    api.mkdir(parents=True, exist_ok=True)
    (api / "latest.json").write_text(json.dumps(release, indent=2))
    (api / "list.json").write_text(json.dumps([release], indent=2))
    print(f"{a.component}: {asset} ({asset.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
