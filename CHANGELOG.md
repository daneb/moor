# Changelog

Notable changes to moor. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and moor uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

### Changed

### Fixed

- **A kiro project's agent can now open its session database.** The sandbox mounted a writable volume for kiro-cli's credential dir (`~/.aws`) but not for its SQLite session store (`~/.local/share/kiro-cli/data.sqlite3`), which fell on the read-only rootfs — so every `moor go` turn blocked with `kiro-cli exited non-zero: Failed to open database … unable to open database file` after a 60s lock wait. A second named volume (`kiro-data`) now backs that path, matching how `.claude` and `.copilot` are handled. Re-render compose with `moor up <project>` to pick it up.

## [0.10.3] - 2026-11-09

### Added

### Changed

### Fixed

- **`moor next` no longer reports a dropped spec as "approved, not shipped yet".** An abandoned spec is `complete: true` just like a merge-approved one, so `Target::unshipped` — filtering on `complete` alone — offered it for `moor ship` and made the guidance advertise shipping a spec that was retired. A new pure `guide::is_unshipped` excludes the abandoned stage, so neither `moor next` nor `moor ship` picks up a dropped spec.
- **`moor drop` now names the spec it will abandon, and why it was chosen, before acting.** Drop targets the active spec, which can differ from the one `moor next` happened to foreground; a silent "dropped X" was how the wrong spec got retired. It now prints the slug, its stage, and the selection reason (only one in progress / most recently worked on / furthest along / pinned) first.

## [0.10.2] - 2026-11-09

### Added

### Changed

### Fixed

- **`moor ui` no longer spins when a project page fails to load.** Opening a
  project URL without the per-launch token (or on any failed fetch) left the
  console firing the same `/api/projects/<p>` request in an unbounded loop —
  thousands of 401s pegging the browser, which looked like a hang. The loader
  now stops re-requesting a view whose last attempt errored, and the project
  page shows a "Could not load" banner with the reason and a Retry button
  (for a missing token: "reopen the URL moor ui printed") instead of a silent,
  looping skeleton.

## [0.10.1] - 2026-11-09

### Added

### Changed

### Fixed

- **A corrupt audit-chain line no longer bricks a project.** `moor next`,
  `moor ui` and every logged `docker exec` read the project's hash chain to
  find the next entry; a single malformed last line made all of them fail
  with `trailing characters at line 1 column N`. The reader now recovers the
  last well-formed entry — reading the final record off a line that holds
  more than one, and walking back past an unparseable line — so one bad line
  degrades gracefully instead of stopping every command.
- **Audit-chain appends are atomic and serialised.** Each entry is written
  as a single line in one `write_all`, and the read-compute-write cycle holds
  an exclusive advisory lock, so two `moor` processes writing at once can no
  longer interleave two records onto one line (the corruption the fix above
  recovers from). See `docs/decisions/0016-*`.
- **`moor up` no longer tries to pull an impossible image.** Setting a
  project's agent and image to disagree (e.g. a kiro-layer image on a copilot
  project) composed a name like `moor/copilot-kiro-python` that is never
  built, so `up` fell through to a registry pull and failed with
  `pull access denied`. `resolve_image` now refuses to stack a second agent
  prefix onto an image already in another agent's layer, and the console's
  settings edit rejects a cross-agent image with a message naming the
  mismatch.

## [0.10.0] - 2026-11-05

- **keel is now an internal detail.** The hidden `moor keel <args>` command is
  `moor pipeline <args>`, the agent's MCP server is `moor-mcp` with tools
  `check` and `next` (was `moor-keel-mcp`, `keel_gate`, `keel_next`), and the
  console and guidance say "pipeline". The sandbox image must be rebuilt
  (`./images/build.sh`) for the renamed MCP binary.
- **README rewritten** and shortened; the detail moved to `CONTRIBUTING.md` and
  `docs/`.
- **keel 0.14.0 is pinned in the sandbox image** (was 0.12.0). It includes the
  fix for a passing run that predates a re-approved spec or plan, which made
  `moor go` skip a rebuild. Rebuild the images with `./images/build.sh`.

## [0.9.4] - 2026-11-03

- **Homebrew cask and macOS binaries.** Each GitHub release now carries
  `keel-<version>-aarch64-apple-darwin.tar.gz` and
  `keel-<version>-x86_64-apple-darwin.tar.gz` (with checksums), and the cask in
  `daneb/homebrew-tap` is updated automatically. Install with
  `brew install --cask daneb/tap/keel`.

### Changed

- Patch and minor dependency updates (five crates).

### 0.9.2

- **`moor drop` works in the sandbox again.** `moor drop` calls
  `keel spec abandon` inside the sandbox, a subcommand keel added in
  0.12.0. The base image still pinned keel 0.11.0, so every sandbox
  carried a keel too old for it and `moor drop` failed with
  `unrecognized subcommand 'abandon'`. The base image now pins keel
  0.12.0. Existing sandboxes need an image rebuild (`make images`) and a
  `moor up` to pick it up.

## [0.9.1] - 2026-11-02

### Added

- **`moor drop`: abandon the active spec.** A guided verb beside
  `moor reject`, built on the same resolver seam. Where reject sends a
  spec *back* for revision by recording a gate rejection, `drop "<why>"`
  retires it for good — it records a terminal abandonment through keel
  (`keel spec abandon`), so the spec leaves the pipeline and stops
  appearing in `moor next`. It refuses an empty reason, needs no
  `--force`, and works at any pre-merge stage. `guide::step_label` now
  reports `abandoned`, and `pick_active` drops an abandoned spec (keel
  reports it `complete`), so a retired spec never becomes the one
  `moor go` points at. (SPEC-0019; depends on keel's abandoned terminal
  state.)

### Changed

- **Retired four stalled draft specs.** `compose-project-namespace`,
  `document-host-deployed-projects`, `agent-session-protocol` and
  `studio-multi-project-console` had passed G0 but were never approved
  or built; with no terminal abandoned state they surfaced in
  `moor next` indefinitely. Moved to `.keel/specs/_abandoned/` as a
  `git mv` — the specs and their G0 evidence stay in history and can be
  revived — with a README recording why each was retired.

## [0.9.0] - 2026-10-27

### Added

- **Run a project's sandbox on the Kiro CLI agent.** moor's turn
  protocol already spoke Kiro; this adds the sandbox beneath it. A
  `--agent kiro` project composes onto a new `moor/kiro-<lang>` image
  layer (`images/kiro/Dockerfile`, opt-in exactly like the Copilot
  layer), and `manifest::resolve_image` now reconciles a kiro image back
  to the kiro agent. The enterprise credential — IAM Identity Center SSO
  tokens under `~/.aws`, left by a host `kiro-cli login` — is discovered
  fresh at each `moor up` and delivered into the sandbox by a
  `kiro-state` volume mounted at `~/.aws`, never through a process
  argument and never copied into the manifest, compose file, or
  Keychain. It is injected only for kiro-agent projects, and its absence
  is a no-op the turn's own auth hint reports. The default-deny egress
  allowlist gains the AWS SSO OIDC and CodeWhisperer host patterns a
  device-flow login and chat turn need. (SPEC-0018)

### Security

- **Patched two fixable HIGH CVEs in the base sandbox image.** `undici`
  6.28.1 → 6.29.1 (CVE-2026-109869, unhandled stream-error DoS) in npm's
  vendored tree, and `github.com/klauspost/compress` → v1.18.7
  (CVE-2026-63209, integer-overflow DoS) in the from-source `gitleaks`
  build — both following the base image's existing in-file patch
  patterns. Surfaced by the CI image scan; applies to every image, since
  they all inherit this base.

## [0.8.0]

### Web UI - 2026-10-18

- **`moor ui`: the console in a browser.** Every project at a glance,
  with the ones waiting on you first; each spec as a map of its seven
  steps plus ship, where every node opens the artefact, gate checks,
  approval history and the action for that step; the agent (brainstorm
  or build) with its recorded transcript and "turn this answer into a
  recipe"; every run's gates, evidence files and trajectory, with
  `moor bundle` a click away; the audit trail as a filterable timeline
  with verify and export; and agent provider, keel driver, Keychain
  credentials, egress allowlist and resources in one settings page.
  Projects can be created and imported (with a repo picker) from it.
  It binds 127.0.0.1 only and lives exactly as long as the command does;
  a 192-bit key printed in the URL's fragment is required on every API
  call and the `Host` header must be loopback. Every action runs the
  `moor` binary itself, from a fixed table of kinds, so the page and the
  command line can't disagree; every sandbox read is logged under
  `kind: "ui"`. `guide::pick_active` now ignores those reads, so looking
  at a spec never redirects `moor go`. No new dependencies: the HTTP
  layer is hand-rolled over `std::net`, like the MCP server over stdio.
  See [ADR-0015](docs/decisions/0015-browser-console.md).

## [0.7.2] - 2026-10-15

### Security

- **The GitHub token no longer enters the sandbox.** Credentialed git —
  `moor ship`'s push, the clone in `moor new`/`moor import`, and `moor
  pull` — now runs on the host with the host's own GitHub credential; the
  sandbox keeps only token-free local git. `GITHUB_TOKEN` is removed from
  the sandbox's compose env and secret manifest. Commits are still made
  in-sandbox (ADR-0007 authorship intact) and leave as a `git bundle`
  streamed over `docker exec`; the host-side push re-emits the same
  `git-push` audit entry (ref + commit). Verified live: `GITHUB_TOKEN`
  absent from a new-binary sandbox's env, present in an old-binary one.
  See [ADR-0014](docs/decisions/0014-git-on-host.md). (SPEC-0016)

### Changed

- **Dropped the unmaintained `serde_yaml` dependency.** `serde_yaml` 0.9
  (deprecated) pulled in `unsafe-libyaml`, flagged by RUSTSEC-2024-0449.
  Swapped to `yaml_serde` 0.10.7 across all 16 call sites; the API is
  compatible, so manifest round-trip and front-matter parsing are
  unchanged and existing tests pass unmodified. `cargo deny` advisories
  now pass and `unsafe-libyaml` leaves the tree. (SPEC-0017)

### Fixed

- **A Copilot sandbox no longer tries to run the Claude CLI.** A
  `copilot`-agent sandbox whose manifest omits `agent:` defaulted the
  field to Claude (a serde default kept so old manifests still load), so
  a turn ran the `claude` CLI inside a Copilot sandbox and died with
  Claude Code's misleading `Not logged in · /login`. `Manifest::agent()`
  now reconciles the stored agent against the image — a defaulted Claude
  on a `moor/copilot*` image resolves to Copilot — and every turn and
  credential read-site routes through it.

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
