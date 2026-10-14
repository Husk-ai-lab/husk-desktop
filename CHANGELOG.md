# Changelog

All notable changes to the Husk AI desktop shell (`husk-desktop`). The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are the app version in
`src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`, which the release workflow
checks against the tag.

This file covers the shell only. The Husk AI and Husk Studio backends are released from their own
repositories (`Husk-ai-lab/HuskAI-core`, `Husk-ai-lab/husk-ai`) and are downloaded by the shell.

Every installer is still **unsigned**: macOS Gatekeeper and Windows SmartScreen warn on first
launch (see the README, "Install" and "Releasing").

## [Unreleased]

Work that exists on a branch but is not on `main` yet.

### Fixed
- Flaky supervisor unit test on the Intel Mac runner (`PortBusy` in
  `incomplete_install_is_reported_as_not_installed`): the test ports now come from below the OS's
  ephemeral range, so other tests' HTTP client sockets cannot collide with them.
  *(branch `ci/test-port-flake`, commit `51abe06`)*

### Added
- `scripts/install-macos.sh` and a README "Install (macOS)" section: a `curl` installer that picks
  the DMG for the Mac's CPU, verifies it against the release's `SHA256SUMS` (refuses on mismatch
  or when the file is missing), installs `Husk AI.app` and unmounts the image. A file fetched by
  `curl` is not quarantined, so the unsigned app opens without the Gatekeeper block. The raw URL
  in the README only works once this is merged to `main`.
  *(branch `docs/curl-install`, commit `a2e5d78`)*

## [0.2.0] - 2026-10-14

Husk's "Install Husk Studio" button now opens the shell's Manager. Needs Husk `0.1.2-alpha.1` or
later for the button to reach the shell; with an older Husk it opens the GitHub page as before.
Commits `b412b20` and the port-test fix.

### Added
- **"Install Husk Studio" in Husk opens the Manager.** The shell long-polls Husk's
  `GET /internal/shell/requests` (launch secret in `X-Husk-Launch-Secret`) once Husk is healthy and
  acts on `open_manager` requests: the Manager opens on the requested tab and the component's card
  (*Add*, *Repair* or *Open Studio*) is highlighted and scrolled into view. A window that does not
  exist yet learns the card from its URL hash, so the first request is never lost. Requests older
  than 30 s and anything outside the allow-lists are ignored; the loop stops after three 403
  answers. One loop exists at a time and a restarted Husk is learned from scratch. No window that
  shows a backend gets IPC. Needs Husk `0.1.2-alpha.1` or later; with an older Husk nothing is
  queued and the click opens GitHub as before.
- `src-tauri/src/bridge.rs` with 13 unit and fake-endpoint tests; `ui/src/focus.ts` with 5 tests;
  the real-backend end-to-end test now also simulates the click and checks that the loop receives
  it (`bridge: passed`, or `unsupported` for a Husk without the endpoint).

### Fixed (tests only)
- `a_port_with_time_wait_sockets_is_lingering_not_in_use` and `an_unused_port_is_free_for_a_strict_bind`
  failed once on the macOS arm64 runner because an ephemeral port they had just released was bound
  by a parallel test. They now use a fixed port range.

## [0.1.3] - 2026-10-14

Fixes Husk Studio failing to start after a quick restart, and makes updates clean up after
themselves. Commit `e07c950`, release commit `ac085da`.

### Fixed
- **Husk Studio exited at start-up with `port 7654 is already in use; --port was given explicitly`**
  when the app was restarted within about a minute of stopping it, although nothing was
  listening. Cause: the shell's port check (Rust, `SO_REUSEADDR`) passed while Studio's own check
  (plain Python `bind`) failed on sockets left in `TIME_WAIT`. Reproduced with the installed
  Studio binary.
- Choosing "Open Husk Studio" while Studio was still starting could start it a second time.
- The rollback target of a component could name a version whose folder no longer exists, so the
  Manager offered a rollback that could not work.
- Husk Studio's `pending_confirm` flag was never cleared after an update.

### Changed
- The shell now probes a backend's port **the way the backend binds it** and waits for it: up to
  60 s while only closing sockets remain, a 3 s grace when a program really holds it, and no wait
  for a reserved range. If a backend still exits with "already in use" it is started once more.
- Husk Studio is started **in the background after Husk**, so a slow or failing Studio never delays
  Husk. Husk is given Studio's URLs whenever Studio is installed (its Studio client probes lazily).
- **Old versions are deleted after an update**: once the new version has started successfully
  once, the version it replaced and any `.tmp` leftovers are removed and the rollback entry is
  cleared, for Husk and for Studio. Until that first successful start the old version is kept for
  the automatic rollback. Previously one old version stayed on disk until the next update, and
  Studio's were never removed.

### Added
- Port errors now say who holds the port: `port 7654 is already in use by <program> (pid N)`
  (`lsof` on macOS/Linux, `netstat` on Windows, best effort).
- New `port_reserved` error for a port the system refuses to give (Windows ranges reserved by
  Hyper-V, WSL or Docker), with its own message in English and Italian, separate from "in use".
- A dialog when Husk Studio cannot start, explaining the reason with *Try again* / *Close*; the
  same dialog when "Open Husk Studio" fails (it used to be only a log line).
- Tests: a real `TIME_WAIT` port, the wait budgets, retry-once and no-retry cases, `lsof` and
  `netstat` output parsing, confirm-and-prune for both components, stale rollback targets.
- README "Ports and quick restarts" section; the "Updates" section describes the new rollback
  window.

### Dependencies
- `socket2` 0.6 (already in the lockfile transitively).

## [0.1.2] - 2026-10-14

Commit `1d97b3c`, release commit `8138b18`.

### Added
- **Update popup at every start.** Once Husk is up, the shell looks for a newer release of itself
  (`Husk-ai-lab/husk-desktop`, latest published release) and of the installed backends, and shows
  one native dialog listing what is newer, in English or Italian. *Update now* downloads and
  SHA-256-verifies the backend update and offers a restart (the new version is used at the next
  start); a newer shell cannot replace itself, so its release page is opened. *Later* closes it and
  asks again at the next start. Nothing is shown offline or when rate limited, and start-up is not
  delayed. It never opens the installation manager.
- `ReleaseSource::latest_shell` (`GET /repos/Husk-ai-lab/husk-desktop/releases/latest`; drafts and
  non-version tags ignored) and a `notify` module for the wording and decisions.
- README "Updates" section.

### Dependencies
- `tauri-plugin-dialog` 2 (native message dialogs, used from Rust only; no window gets a new IPC
  permission).

## [0.1.1] - 2026-10-13

CI/CD and test infrastructure; no change to what the app does. Release commit `fd5dd41`.

### Added
- **Release workflow** `.github/workflows/release.yml` (`desktop-shell`), the same process as the
  backend's: `verify` → native build matrix → `report` → draft release on a tag.
  - Triggers: push to `main` (builds, no release), tag `v*` (builds, then a **draft** release),
    manual run (never releases).
  - `verify`: version gate (`tauri.conf.json`, `Cargo.toml`, `package.json` and the tag agree),
    script tests, UI build, `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`.
  - Native builds: macOS arm64 (`macos-14`), macOS Intel (`macos-15-intel`), Windows x64 (NSIS
    `.exe`), Linux x64 (`ubuntu-22.04`, AppImage and `.deb`). Required targets: both Macs and
    Windows; Linux is built and reported but does not gate.
  - Unit tests run on each OS in its build job; installers are named
    `Husk-AI_<version>_<triple>.<ext>`, hashed, and (macOS) the DMG is verified and its app
    version checked.
  - `report` writes `shell-release-report.json`/`.md` and `SHA256SUMS` even when a target fails,
    then fails; `release` runs only for a tag and only when the report passed.
  - Every action is pinned by commit SHA; the top-level permission is `contents: read` and only the
    release job has `contents: write`; signing secrets reach the signing step only and signing is
    skipped when they are absent (the report says so).
- `scripts/release.py` (`check-version`, `collect`, `report`) with 18 unit tests, usable on a
  laptop and in CI.
- **First-run end-to-end test** (`src-tauri/src/first_run_e2e.rs`, `#[ignore]`, needs
  `HUSK_E2E=1`): on a clean app-data folder it resolves the published backend release for the
  runner's OS/CPU, downloads and SHA-256-verifies it, installs it, starts it exactly as `launch`
  does, checks `/health` and its version, and stops it. It reports `skipped` on a platform the
  backend does not publish (Linux today). Passed on macOS arm64, macOS Intel and Windows against
  `husk-backend` `0.1.0-alpha.1`.
- README "Releasing" section describing the triggers, the required targets and how to cut a
  release.

### Changed
- Source formatted with `rustfmt` and five `clippy` findings fixed (no behaviour change).

### Fixed (tests only)
- Windows: the installer test fixtures used `husk-backend` without `.exe`, so four tests failed
  there.
- Supervisor tests could be given the same port by the OS when run in parallel.
- The supervisor test backend (Python `http.server`) did a reverse-DNS lookup before listening,
  which took longer than the test's timeout on the Intel Mac runner.

## [0.1.0] - 2026-10-12

First release: a hand-built, unsigned macOS Apple Silicon DMG (`Husk-AI_0.1.0_aarch64.dmg`).
Commits `778d8ad` and `c885613`.

### Added
- **The shell** (Tauri 2, Rust + React UI): an empty installer shell that, on first launch, asks
  what to install (Husk AI is required, Husk Studio is optional and recommended), downloads the
  PyInstaller one-folder backends from GitHub Releases, starts them on loopback and shows their
  web UIs in native windows (Husk in the main window on port 8765, Studio in its own window on
  port 7654). The shell never logs in, never calls an AI provider, Neon or Stripe, and never reads
  documents.
- **Release discovery**: GitHub Releases as the source, behind a `ReleaseSource` trait so the
  gateway `GET /v1/status` can replace it later. Assets are
  `<husk-backend|husk-studio>-<version>-<target triple>.(tar.gz|zip)` for
  `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc` and
  `x86_64-unknown-linux-gnu`. Stable channel uses `releases/latest`, beta lists recent releases;
  drafts are ignored; a real version replaces a non-version tag such as `alpha-release`.
- **Install pipeline**: download with resume and size check → **SHA-256 check against the
  `digest` GitHub publishes for the asset** (a release without a checksum is refused, never
  installed unchecked) → extract (tar.gz or zip, path-escape safe) → locate the PyInstaller root →
  executable bit → atomic rename → `config.json` written last. An install is intact only with the
  executable and `_internal/` (or a libpython file); a broken one is reported as such and offered
  *Repair*.
- **Lifecycle**: install, update (downloaded next to the current version, applied at restart),
  repair, rollback to the previous version, removal of Studio (data folder untouched), update
  check at start with a menu label.
- **Process supervision**: backends start on 127.0.0.1 with the launch secret and data folder,
  Husk with `--data-dir <app-data>/data/husk`, Studio with
  `start --port 7654 --host 127.0.0.1 --no-open-browser`; `/health` wait (20 s) with early-exit
  detection; SIGTERM then kill; leftover processes from a previous run are reaped through pid
  files; a Windows job object and Linux `PDEATHSIG` stop children with the shell. A PyInstaller
  "Failed to load Python shared library" is reported as an incomplete install, not a crash.
- **Windows and navigation**: external links open the system browser, links to Studio open the
  Studio window; the Studio window has a *Back to Husk* button and a *Go to Husk AI* menu item
  (`Cmd/Ctrl+Shift+H`); the Manager window shows components, updates and logs; Italian and English.
- **Packaging**: DMG layout (660×400, app and Applications link), ad-hoc signature, macOS 11+.
- **Test tooling**: `scripts/make_local_release.py` and `scripts/serve_local_release.py` simulate
  GitHub Releases (with digests, resume and failure injection) for manual testing; 58 unit tests.
- README with the architecture contract, the release checklist and the backend contracts the shell
  relies on.

### Known limitations at this release
- Not signed with a Developer ID and not notarised; macOS Apple Silicon only.
- Authenticity is not proven: the SHA-256 check shows the file is the one the release holds, but
  there is no signed manifest.
- The shell cannot update itself.

[Unreleased]: https://github.com/Husk-ai-lab/husk-desktop/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/Husk-ai-lab/husk-desktop/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/Husk-ai-lab/husk-desktop/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/Husk-ai-lab/husk-desktop/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/Husk-ai-lab/husk-desktop/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Husk-ai-lab/husk-desktop/releases/tag/v0.1.0
