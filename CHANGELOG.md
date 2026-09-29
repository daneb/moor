# Changelog

Notable changes to moor. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and moor uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-09-29

The first release with a guided workflow: at every step, moor says where
the active spec stands and the one command that moves it on.

### Added

- **`moor next`** shows the active spec's step ("step 5 of 7: approve the
  plan"), in plain words, and the one command to run, runnable as shown.
  `moor next --all` lists every spec; `moor use <project> --spec <name>`
  pins which one it follows.
- **`moor go`, `moor approve`, `moor reject "why"`** act on the active spec,
  so you never type a spec name, a stage or a pipeline command. `go` takes
  the automatic steps until one needs you; `approve` shows the spec, plan or
  run first, then asks.
- **`moor spec new <name>` and `moor spec push <file>`**: write a spec on
  your Mac (by hand or with an AI assistant; the rules are at the top of the
  starter file), then send it into the sandbox, where it is checked and made
  the active spec. Only the spec's text crosses, host to sandbox.
- **Rejections are shown**: who rejected an approval, why, and how to revise
  it. `moor go` re-checks a rejected or stale approval once. Needs keel
  0.11.0, which the images now pin.
- **One evidence record**: moor writes its audit trail in the pipeline's own
  hash-chained format, folds the pipeline's entries in from outside the
  sandbox, and attests the running sandbox (read-only, non-root, no
  capabilities, no mounts) on `moor up`.
- **`moor bundle`**: a verified evidence bundle carrying moor's audit trail,
  built and checked in throwaway containers with no network.
- Approvals record your git identity instead of "unknown".
- `cargo-audit` and `gitleaks` in the images, so advisory and secret checks
  run in the sandbox instead of reporting "blocked".

### Changed

- **The pipeline stays behind the scenes.** `moor --help`, moor's messages,
  the README and the site describe what happens, not which keel command runs.
  `moor keel <args>` still works as a hidden escape hatch; what moor is
  built from is in docs/ARCHITECTURE.md.
- `moor --help` lists commands in the order a project is worked, each with a
  one-line summary.
- `moor new`, `moor import`, `moor up` and `moor use` end on a "Next:" line;
  the pipeline's own setup output is shown only if setup fails.
- A paused `moor recipe` points at `moor approve` / `moor reject`, and makes
  its spec the active one.
- `moor bundle` writes `moor-<project>-….tar.gz` (was `keel-…`).
- Images pin keel 0.11.0.

### Fixed

- Rust projects can build in the sandbox: cargo's registry now lives on the
  project's cache volume (the root filesystem is read-only), and clippy and
  rustfmt are installed.
- `/tmp` in the sandbox is executable, so test suites that run a script from
  a temp directory pass.
- gitleaks is built from source with a current Go toolchain, so the image
  scan passes.

### Removed

- The rules-based command gate on `moor run` (briefly added and reverted in
  this cycle). It filtered only the commands the operator typed, never the
  agent's, and guarded nothing on the host; see
  docs/decisions/0009-no-command-gate.md.

### Upgrading

Rebuild the images (`./images/build.sh`), then `moor up` each project so it
picks up the new sandbox settings. Projects keep the keel their image was
built with until then.

## [0.2.1] - 2026-09-14

Published to crates.io before this changelog existed.
