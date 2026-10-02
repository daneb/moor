# Changelog

Notable changes to moor. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and moor uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.0] - 2026-10-02

### Added

- **`moor studio` is now enough to run a project.** The console gained the
  two things that previously sent you back to the CLI mid-flow:
  - **reject** — `x` starts a rejection, you type the reason, `Enter`
    arms it and a second key confirms (`Esc` abandons, an empty reason is
    refused). The same armed two-key shape approval already had, so no
    single keystroke records a decision.
  - **a failing gate's checks, in place** — when a spec sits at a gate
    that failed, the checks it failed are listed under the project, read
    from keel's own gate record, so you can see what to fix without
    leaving. (SPEC-0013)
- **Turn a studio conversation into a spec.** `/` takes the agent's latest
  answer and writes it out as a recipe draft, then names
  `moor recipe <project> <file>` as the next step. The answer goes through
  the same parse-before-write guard `moor ask --emit-recipe` uses, so an
  answer that is not a valid recipe is kept as a `.draft` to fix rather
  than installed or lost. (SPEC-0014)
- **moor's own agent turns run on any configured agent.** `moor ask`,
  `moor studio` and `moor recipe`'s authoring step were hardcoded to
  Claude; they now drive **claude, copilot or kiro** from one profile
  table (per-agent invocation, output shape, resume form and credential
  hint), and the `agent-turn` chain entry names which agent produced it.
  A project made with `--agent copilot` no longer builds with Copilot but
  converses with Claude.
  **Safety note:** kiro exposes only `--trust-tools` (auto-approval),
  which [ADR-0008](docs/decisions/0008-agent-session-protocol.md) showed
  does not restrain an agent, and no deny flag — so a role that withholds
  tools is **refused** on kiro rather than run with every tool. Claude and
  Copilot pass their own deny flags. (SPEC-0012)
- **Seamless Copilot auth from a host `copilot /login`.** For a
  `copilot`-agent project, `moor up` discovers the OAuth device-flow token
  the Copilot CLI stored in the macOS Keychain (service `copilot-cli`) and
  injects it as `COPILOT_GITHUB_TOKEN` — so logging in once on the host is
  all it takes, with no personal access token to create or `moor secrets`
  step. An explicit env/Keychain credential still wins; non-copilot
  projects are untouched; the token is read fresh each `up` and never
  persisted. (First host read of Copilot's Keychain item triggers a
  one-time macOS "Always Allow" prompt.) See
  [ADR-0012](docs/decisions/0012-copilot-device-flow-auth.md).

### Changed

- **Verified bundles are committed; raw run evidence is not.**
  `.keel/runs/` was 12M of a 13M `.keel/` and growing quadratically — a
  run records the working-tree diff, so while runs were themselves
  tracked, every run's diff contained the previous runs' evidence
  (measured: `diff.patch` went 63K → 333K → 508K → 1051K → 2150K in one
  afternoon). `.keel/runs/` and `.keel/chain.jsonl` are now ignored and
  `.keel/bundles/` is tracked, which is what `keel cover` already assumed:
  a bundle is 232K against a 1.1M raw run and verifies on its own.
  `make evidence` writes one and verifies it, failing rather than leaving
  an unverifiable bundle to commit. Per-spec gate records and approvals
  stay tracked, so what was *decided* is still in git. This also removes a
  recurring merge conflict: the chain is append-only and hash-linked, so
  parallel branches always conflicted in it and resolving always discarded
  one side's entries. Rationale, numbers and the migration are in
  [docs/EVIDENCE.md](docs/EVIDENCE.md). (SPEC-0015)

## [0.6.0] - 2026-10-01

### Added

- **`moor doctor`** shows the sandbox workspace's git state before you
  build on it — which branch it's on (and whether that's the trunk),
  clean vs. a count of uncommitted changes, ahead/behind the remote, and
  detached HEAD — spelling out each concern a build would care about. It
  is read-only: no fetch, no checkout, it changes nothing (refreshing is
  `moor pull`). First of a set: a hard build gate and spec archiving (as a
  recorded, reversible state) are planned on top of it. See
  [ADR-0011](docs/decisions/0011-sandbox-git-visibility-and-guards.md).

## [0.5.3] - 2026-10-01

### Added

- `moor --version` / `moor -V` now report the version (sourced from the
  crate version, so it always matches the released build). Previously the
  CLI had no version flag and rejected `--version` as an unknown argument.
- **`moor pull`** refreshes the sandbox's current branch from its GitHub
  remote over HTTPS, with a token injected inside the container at
  runtime. It works even though the sandbox has no SSH client (so a plain
  `git pull` against an SSH `origin` fails with "cannot run ssh"): it
  derives an `https://github.com/...` URL from `origin` in any form
  (scp-style, SSH host alias like `github.com-sbg`, `ssh://`, or already
  HTTPS), reads `$GITHUB_TOKEN` via a credential helper so it never
  appears in a host-side argv, leaves `origin` unchanged, and persists no
  credential to `.git/config`. See
  [ADR-0010](docs/decisions/0010-sandbox-git-transport.md).

### Fixed

- `moor ship` said "Nothing to ship" for a spec that `moor next` was
  pointing at with "approved, not shipped yet". When more than one spec
  had unshipped work, `ship` chose via the active-spec logic, which only
  surfaces a pin while the spec is still in progress — so once a build was
  approved (making the spec `complete`), the pinned spec was dropped and
  `ship` found nothing, even though `next` still guided you to it. `ship`
  now consults the pin directly and ships it whenever it is among the
  unshipped specs, keeping the two commands in agreement.
- `moor reject` did nothing once a spec reached the build step ("step 6 of
  7: build it"): it reported "Nothing is waiting for your decision" because
  the build step is not an approval gate, leaving a spec with no guided way
  to be pulled back after its plan was approved. `moor reject "why"` at the
  build step now rejects the approved plan that launched the build (with
  the reason recorded), moving the spec back out of the build step — the
  honest meaning of "I don't want this built after all". Rejection at the
  spec/plan/merge gates is unchanged.

## [0.5.2] - 2026-10-01

### Fixed

- `moor next` (and the guidance footer of `moor go`) ignored an explicit
  active-spec pin whenever another spec had approved-but-unshipped work:
  it always steered you to ship the unshipped spec, so `moor use <p>
  --spec <slug>` had no effect on which spec the guidance followed. An
  explicit pin now wins — the pinned in-progress spec is guided, and the
  unshipped work is shown as a secondary "Also:" note rather than
  hijacking the step. Without a pin, unshipped-first is unchanged.

## [0.5.1] - 2026-09-30

### Fixed

- The GitHub Copilot in-sandbox agent (shipped in 0.5.0) did not actually
  work end-to-end. Two bugs, both fixed and verified live against a real
  SBG enterprise Copilot licence:
  - The `moor/copilot` image had no writable `~/.copilot`, so the Copilot
    CLI died silently under the read-only rootfs. The image now creates
    `~/.copilot` agent-owned and the compose template mounts a
    `copilot-state` named volume there — the same mechanic as
    `~/.claude`. (Rebuild the CLI so the compiled-in compose template
    updates, then recreate the project.)
  - `*.githubcopilot.com` (the Copilot model API) was in the allowlist
    source but the `moor/egress` image had not been rebuilt, so egress
    blocked it. Rebuilding the image picks it up.

## [0.5.0] - 2026-09-30

### Added

- **GitHub Copilot as an opt-in in-sandbox build agent.** keel's build
  step inside the sandbox can now run GitHub Copilot instead of Claude
  Code, selected per run with `keel run --driver copilot`. Copilot ships
  in its own image layer, `moor/copilot` (build a project on it with
  `moor new <name> --image moor/copilot:latest`), so only projects that
  opt in carry its ~174MB agent binary — the base and language images are
  unchanged. Copilot auth is injected from the host like Claude's, never
  baked into the image (`COPILOT_GITHUB_TOKEN` > `GH_TOKEN` >
  `GITHUB_TOKEN`), and `*.githubcopilot.com` is on the egress allow-list.
  moor's own agent commands (`moor ask`, `moor studio`, `moor recipe`)
  remain Claude-specific. See [docs/IMAGES.md](docs/IMAGES.md).

### Fixed

- The base image now patches two CVE-carrying packages vendored inside
  npm's own dependency tree (`brace-expansion`, `undici`) that no released
  npm version has picked up the fix for yet, dropping the patched releases
  straight into npm's tree. Every image inherits the fix; the Trivy CVE
  gate stays strict, with no ignore file.

## [0.4.0] - 2026-09-30

### Added

- **`moor ship`** takes a spec whose merge you approved to GitHub: it shows
  the files it will commit (the change, the spec's folder and its runs,
  nothing of other specs), asks, commits them on `moor/<spec>` in the
  sandbox, pushes from the sandbox, opens the pull request from your Mac
  with `gh`, and returns the sandbox to its trunk branch for the next spec.
  `moor next` says when approved work is waiting to ship.
- At the merge step, `moor approve` shows what the build changes and the
  checks its run passed, instead of every run's history; `moor view <spec>
  diff` shows the full change.

- **`moor go --check`** re-checks the build step without the agent, for
  work you fixed by hand or with `moor ask`. After a merge rejection, the
  guidance points at it instead of a full rebuild.
- `moor view <spec> report` shows every check each of the spec's runs
  passed or failed.

### Fixed

- A failed build now ends with what to do: fix what failed (a `moor ask`
  pointed at the run's evidence), re-check with `moor go --check`, or let
  the agent rebuild, instead of only "Next: moor go".
- A build that passes and reaches the merge approval is no longer reported
  as an error with a failing exit code.
- `moor new` makes the workspace a git repository with a first commit. A
  new project had neither, so its first build could never run.
- `git stash push` (and any command with "push" after another git
  subcommand) is no longer recorded in the audit trail as code leaving the
  sandbox; only git's actual subcommand counts, after its global options.

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
