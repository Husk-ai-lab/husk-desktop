#!/usr/bin/env python3
"""Serve .local-release/ like GitHub does, with Range support (so download resume can be tested).

    python3 scripts/serve_local_release.py [--port 8800] [--throttle KBPS] [--fail-after BYTES]

  GET /api/repos/<owner>/<repo>/releases/latest   -> api/<owner>/<repo>/latest.json
  GET /api/repos/<owner>/<repo>/releases?...      -> api/<owner>/<repo>/list.json
  GET /download/<asset>                           -> download/<asset> (Range supported)

--throttle slows the body down so the progress bar is visible; --fail-after drops the connection
once per asset after that many bytes, to exercise resume.
"""
from __future__ import annotations

import argparse
import re
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / ".local-release"
args: argparse.Namespace
failed_once: set[str] = set()


class Handler(BaseHTTPRequestHandler):
    def _send_file(self, path: Path, ctype: str, ranged: bool) -> None:
        data_len = path.stat().st_size
        start = 0
        m = re.match(r"bytes=(\d+)-", self.headers.get("Range", "")) if ranged else None
        if m:
            start = int(m.group(1))
            if start >= data_len:
                self.send_response(416)
                self.end_headers()
                return
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{data_len - 1}/{data_len}")
        else:
            self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(data_len - start))
        self.send_header("Accept-Ranges", "bytes")
        self.end_headers()
        sent = 0
        with path.open("rb") as f:
            f.seek(start)
            while chunk := f.read(64 * 1024):
                if args.fail_after and path.name not in failed_once and start + sent >= args.fail_after:
                    failed_once.add(path.name)
                    self.log_message("dropping connection after %d bytes", start + sent)
                    self.close_connection = True
                    return
                self.wfile.write(chunk)
                sent += len(chunk)
                if args.throttle:
                    time.sleep(len(chunk) / (args.throttle * 1024))

    def do_GET(self) -> None:
        p = self.path.split("?")[0]
        m = re.fullmatch(r"/api/repos/([^/]+/[^/]+)/releases(/latest)?", p)
        if m:
            f = ROOT / "api" / m.group(1) / ("latest.json" if m.group(2) else "list.json")
            if f.is_file():
                return self._send_file(f, "application/json", False)
            self.send_response(404)
            self.end_headers()
            return
        if p.startswith("/download/") and "/" not in p[len("/download/"):] and ".." not in p:
            f = ROOT / "download" / p[len("/download/"):]
            if f.is_file():
                return self._send_file(f, "application/octet-stream", True)
        self.send_response(404)
        self.end_headers()


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=8800)
    ap.add_argument("--throttle", type=int, default=0, help="KB/s, 0 = unlimited")
    ap.add_argument("--fail-after", type=int, default=0, help="drop the connection once after N bytes")
    args = ap.parse_args()
    print(f"serving {ROOT} on http://127.0.0.1:{args.port}")
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()
