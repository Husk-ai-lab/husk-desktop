# husk-desktop

The Tauri 2 shell of Husk AI. The OS installer ships only this empty shell. On first launch it
asks what to install (**Husk AI** is required, **Husk Studio** is optional and recommended),
downloads the PyInstaller one-folder backends from GitHub Releases, starts them on loopback, and
shows their web UIs (FastAPI serving React + Vite) in native webview windows.

The shell never logs in, never calls the AI provider, Neon or Stripe, and never reads documents.
Architecture source of truth: `Company_memory/HuskAI/md/Technical plan for Husk AI.md`.

| Component | Repo (releases) | Binary | Port |
| --- | --- | --- | --- |
| Husk AI (required) | `Husk-ai-lab/HuskAI-core` | `husk-backend` | 8765 (fixed: the Clerk redirect URI is `http://127.0.0.1:8765/callback`) |
| Husk Studio (optional) | `Husk-ai-lab/husk-ai` | `husk-studio` | 7654 (MCP 7655) |

Release assets must be named `<husk-backend|husk-studio>-<version>-<target triple>.(tar.gz|zip)`
(plan, *GitHub Releases plan*), triples `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`. The archive holds the PyInstaller folder
either flat or inside one top folder, and must contain the **whole** one-folder build: the
executable *and* `_internal/` (it holds `libpython`). A bare executable asset is not accepted: it
cannot start, and an install without `_internal/` is reported as broken ("Repair installation").
The tag may be `v1.4.0` or a plain name such as `alpha-release`; a real version replaces it.

## Develop

Needs Rust (stable), Node 20+, Python 3 (test scripts only).

```bash
npm install && npm --prefix ui install
cargo test --manifest-path src-tauri/Cargo.toml     # core logic, no GUI

# try the whole flow against a local "GitHub" (debug builds only; a release build always uses GitHub)
python3 scripts/make_local_release.py --fake --version 0.1.0                       # fake Husk
python3 scripts/make_local_release.py --fake --component studio --version 0.1.0    # fake Studio
python3 scripts/serve_local_release.py --throttle 400                              # terminal 1
HUSK_DESKTOP_APP_DATA=/tmp/husk-shell-test \
HUSK_DESKTOP_GITHUB_API=http://127.0.0.1:8800/api npm run tauri dev                # terminal 2
```

With a real build, replace `--fake` by `--source <HuskAI-core>/dist/husk-backend` (from
`uv run python installer.py`). `HUSK_DESKTOP_APP_DATA` keeps tests away from a real installation in
`~/Library/Application Support/ai.husk.desktop`.

Useful faults: `serve_local_release.py --fail-after 300` drops the connection once to see resume;
truncate an archive in `.local-release/download/` to see "Download non valido"; occupy the port
(`nc -l 8765`) to see the port error.

## Preview the screens without Tauri

`npm --prefix ui run dev`, then open `http://localhost:1420/?view=setup` in a browser (dev builds mock the
shell commands; views: `setup`, `install`, `launch`, `error`, `manager`; options `&tab=updates|logs|settings`,
`&lang=en|it`, `&theme=light|dark`). The mocks are not part of production builds.

## Layout

```
src-tauri/src/  config (compiled-in constants) · paths · platform · state (config.json)
                release/ (ReleaseSource trait, GitHub impl) · install/ (download, SHA-256 verify, extract, installer)
                process/ (supervisor, ports, reaper) · nav (link policy) · windows · app (commands) · logs
ui/             the shell's own screens (setup, install progress, launch, errors, manager),
                styled after Company_memory/HuskAI/pdf/Husk_AI_Prototipo_Schermate.pdf
assets/         logo source (ui/public/brand/ holds the cropped exports; icons come from it)
scripts/        make_local_release.py, serve_local_release.py (fake GitHub for manual tests),
                release.py (CI steps: version gate, installer collection, report) + test_release.py
```

On disk (`<app-data>` = `ai.husk.desktop`): `config.json`, `bin/<component>/<version>/`,
`data/husk|studio/`, `downloads/`, `run/` (pid files), `logs/`.

## Release checklist (BLOCKERS marked)

- [x] SHA-256 check: every downloaded archive is hashed and compared with the `digest` GitHub
  publishes on the release asset (`sha256:<hex>`) before anything is extracted. A release with no
  usable digest is refused, never installed unchecked. This proves the file is the one the release
  holds (no corruption, truncation or swap); it does not stop someone who can edit the release
  itself.
- [ ] **BLOCKER for a wide public rollout: authenticity.** A signed manifest (plan rule 5: minisign
  over the checksums, public key compiled into the shell) is still missing. Implement it behind
  `install::verify::Verifier`.
- [ ] Shell self-update (`tauri-plugin-updater`) is not wired: needs a signing keypair and the
  `latest.json` produced by `tauri-action`.
- [x] CI/CD (`.github/workflows/release.yml`): verify, native build per OS, first-run test, report,
  draft release on a tag. Run on a branch with `workflow_dispatch`; see "Releasing".
- [ ] Signing secrets in GitHub Actions (Apple Developer ID + notarisation, Windows Authenticode).
  The workflow already signs when `APPLE_CERTIFICATE` exists and reports `signed`/`notarised`.
- [ ] Source of versions: switch `ReleaseSource` to the gateway `GET /v1/status` once it exists.

## Releasing

`.github/workflows/release.yml` (workflow `desktop-shell`) is the same process as the backend's
`desktop-build`:

| Trigger | Result |
| --- | --- |
| push to `main` touching `src-tauri/**`, `ui/**`, `package*.json`, `scripts/**` or the workflow | verify, build every OS, first-run test, upload installers and the report as workflow artifacts. **No release.** |
| tag `v<version>[-suffix]` | the same, then a **draft** release with the installers, `SHA256SUMS` and the report as notes. A suffix makes it a pre-release. |
| manual run (Actions tab, or `gh workflow run release.yml --ref <branch>`) | same as `main`; never creates a release |

Jobs: `verify` (version gate, script tests, UI build, `cargo fmt --check`, `cargo clippy -D warnings`,
`cargo test`) then `build` on native runners (macOS arm64 `macos-14`, macOS Intel `macos-15-intel`,
Windows `windows-latest`, Linux `ubuntu-22.04`), each running the unit tests, building the bundles
and the **first-run test**, then `report`, then `release` (tags only; the only job with
`contents: write`).

The first-run test (`src-tauri/src/first_run_e2e.rs`) takes a clean app-data folder, resolves the
backend release published on GitHub for that OS and CPU, downloads and SHA-256-verifies it,
installs it, starts it exactly as `launch` does, waits for `/health`, checks the reported version
and stops it. It runs on each runner's own CPU. It does not start the packaged GUI app. Linux
reports `skipped` while the backend publishes no Linux build.

```bash
# the same test on your machine (stop `tauri dev` first: Husk needs port 8765)
HUSK_E2E=1 cargo test --manifest-path src-tauri/Cargo.toml first_run -- --ignored --nocapture
# the CI helper steps
python3 scripts/release.py check-version          # tauri.conf.json == Cargo.toml == package.json
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Required targets are `REQUIRED_TRIPLES` at the top of the workflow (macOS arm64, macOS Intel,
Windows x64). Linux is built and reported but does not gate; promote it when the backend ships a
Linux build. The report is always written; the `report` job then fails when a required target
failed or is missing, and a tag then produces no draft.

To cut a release:

```bash
# 1. bump the version in src-tauri/tauri.conf.json, src-tauri/Cargo.toml and package.json
#    (the gate fails the run if the three or the tag disagree), update Cargo.lock
# 2. merge to main, check the run is green, then
git tag v<version> && git push origin v<version>
# 3. read the draft release and its report, then publish it
```

Signing is gated on secrets (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`): without them the
macOS bundles are ad-hoc signed and not notarised, so Gatekeeper blocks a downloaded DMG at first
launch (right-click, Open, or `xattr -dr com.apple.quarantine "/Applications/Husk AI.app"`). The
report lists this under known gaps. Windows Authenticode is not wired.

`v0.1.0` was built by hand (`APPLE_SIGNING_IDENTITY=- npm run tauri build -- --bundles dmg`) and
already exists, so the first release made by CI must carry a new version.

## Contracts the shell relies on in the backends

Husk: `HUSK_PORT`, `HUSK_DATA_DIR`, `HUSK_LAUNCH_SECRET`, `HUSK_ENV=prod`, `HUSK_STUDIO_UI_URL`,
`HUSK_STUDIO_OTLP_URL`, `HUSK_STUDIO_MCP_URL`; `GET /health`. Studio (when it ships a binary):
`HUSK_PORT`, `HUSK_DATA_DIR`/`HUSK_HOME`, `HUSK_LAUNCH_SECRET`, `STUDIO_TOKEN_URL`,
`HUSK_NO_BROWSER=1`; `GET /health`. Open items in the backends: parent-death exit (the shell kills
children itself on Windows/Linux and cleans leftovers via pid files on macOS), and `LocalGuard`
accepting the webview origin `http://127.0.0.1:8765`.
