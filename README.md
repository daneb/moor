# moor

Containerized, [keel](https://github.com/daneb/keel)-driven sandboxes for
AI coding agents. Every project gets its own Docker sandbox with no host
bind mount, no `docker.sock`, and a default-deny egress proxy — the agent
(Claude Code today) and keel both run entirely inside the container, so a
misbehaving agent or a malicious cloned repo can compromise at worst one
throwaway container, never the Mac Mini it runs on.

See [docs/THREAT-MODEL.md](docs/THREAT-MODEL.md) and
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design,
[docs/MCP.md](docs/MCP.md) for the transport in both directions — how moor
puts an instruction in front of the agent, and how the agent reaches keel
through an MCP server that deliberately has no `approve` verb in it —
[docs/IMAGES.md](docs/IMAGES.md) for what is in each image, what is pinned
and what isn't, and the build mechanics that are easy to get wrong,
[docs/WALKTHROUGH.md](docs/WALKTHROUGH.md) for a real ideation-to-shipped
run against an actual GitHub repo — including the three bugs it found —
[docs/MIGRATING.md](docs/MIGRATING.md) for bringing an existing project
(tested against keel's own repo) into a sandbox, and
[docs/decisions/0001-container-runtime-choice.md](docs/decisions/0001-container-runtime-choice.md)
for why this stays on `runc` rather than gVisor or Apple's native
`container` tool, and
[docs/decisions/0002-claude-code-authentication.md](docs/decisions/0002-claude-code-authentication.md)
for why Claude Code auth is an injected token rather than a mounted
`~/.claude`,
[docs/decisions/0003-claude-code-permissions.md](docs/decisions/0003-claude-code-permissions.md)
for why every project defaults to Claude Code's own `auto` permission
mode instead of `--dangerously-skip-permissions`,
[docs/decisions/0004-rename-to-moor.md](docs/decisions/0004-rename-to-moor.md)
for why this was `isolator` and is now `moor`,
[docs/decisions/0005-recipe.md](docs/decisions/0005-recipe.md) for
`moor recipe` — driving keel's spec/gate/plan/gate/run pipeline from a
loosely-described outcome, stopping for human approval at the same
checkpoints keel already defines —
[docs/decisions/0006-recipe-logs.md](docs/decisions/0006-recipe-logs.md)
for `moor logs` — a live, timestamped status view of a running recipe
from any terminal, not just the one driving it —
[docs/decisions/0007-git-identity.md](docs/decisions/0007-git-identity.md)
for why the sandbox's git identity is synced from the host's, so
`keel approve` records a real name instead of "unknown",
[docs/decisions/0008-agent-session-protocol.md](docs/decisions/0008-agent-session-protocol.md)
for why the agent reaches keel through moor's own MCP server rather than a
scoped shell with a hand-maintained verb allowlist — and
[docs/examples/ascii-banner](docs/examples/ascii-banner) for a small
utility built end to end by a real Claude Code agent running inside a
sandbox — including two real bugs that run found and fixed, and the
full security/audit verification against the live container.

[docs/CI.md](docs/CI.md) covers what runs in `.github/workflows/ci.yml`
on every push/PR — Rust lint/test/audit/deny, shellcheck, hadolint,
gitleaks, a Trivy CVE scan of every built image, and the real
`tests/e2e.sh` against live containers — and, importantly, what each of
those actually found and fixed versus what's deliberately (and
narrowly) suppressed, with the reasoning inline.

## Quick start

```bash
# one-time: build the sandbox + egress images
./images/build.sh

# one-time: build the moor CLI
cd cli && cargo build --release
# (or use ./target/debug/moor during development)

# create a project (add --github to also create a private GitHub repo)
moor new my-app --image moor/node:latest

# ...or bring an existing project in — history and all, no bind mount
moor import keel --from ~/Repos/keel

# work inside it
moor shell my-app
moor run my-app -- keel status

# keel ships in every sandbox, so once a project is up, `moor keel <args>`
# runs it there directly — no need to name the project again
moor keel status
moor keel gate g1 greet-name

# read a spec's plan/spec/tasks markdown without opening a shell
moor view greet-name tasks

# `moor keel`/`moor view` figure out which project you mean the same way:
# an explicit --project flag, then `moor use <name>` if you've set one,
# then whichever project's sandbox is actually up — only if none of those
# resolve to exactly one project do they ask you to disambiguate. Either
# way, they always print which project they picked first, e.g.:
#   ==> project: my-app (only sandbox currently up)
# so a stale `moor use` default can never silently touch the wrong
# container without you noticing.

# check the sandbox is actually locked down the way it should be
moor selftest my-app

# see what's happened in this project so far
moor audit my-app

# a keel evidence bundle for the latest run, carrying this chain, verified
moor bundle -p my-app --out /tmp

moor down my-app
```

Secrets (`ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`, `GITHUB_TOKEN`,
...) are read from your shell's environment if exported, otherwise from
the macOS Keychain — set one once and every future `moor up` just
picks it up, nothing to re-export each session:

```bash
moor secrets set my-app ANTHROPIC_API_KEY   # prompts, hides input where possible
moor secrets status my-app                  # where each declared secret resolves from
moor secrets unset my-app ANTHROPIC_API_KEY
```

If you pay for Claude via a claude.ai subscription rather than metered
API credits, use `CLAUDE_CODE_OAUTH_TOKEN` instead of `ANTHROPIC_API_KEY`
— the `claude` CLI inside the sandbox honors it the same way. Generate it
once on the host (this does the interactive login there, not in the
container) and store it the same way as any other secret:

```bash
claude setup-token                                    # one-time, on the host — opens a browser login
moor secrets set my-app CLAUDE_CODE_OAUTH_TOKEN   # paste the token it prints
```

Note: whether `keel` invokes the `claude` driver (vs. falling back to
`--no-driver` mode) is keel's own credential check, not moor's — see
[keel](https://github.com/daneb/keel)'s docs if it doesn't pick up
`CLAUDE_CODE_OAUTH_TOKEN` the same way it picks up `ANTHROPIC_API_KEY`.

## Driving keel from a recipe

Typing out `keel spec new` → author it → `gate g0` → `approve` →
`plan` → author it → `gate g1` → `approve` → `run` → `approve --stage
merge` by hand, every time, gets old. `moor recipe` drives that whole
sequence from one loosely-described outcome, stopping at the same human
checkpoints keel already defines:

```bash
moor recipe my-app docs/examples/recipe/greet-function.recipe.md
```

The recipe file is just YAML front matter (`slug`, `scope`) followed by
free text describing what you want — not a DSL, deliberately: the free
text goes to the agent close to verbatim, so writing it loosely is the
intended way to use this, not a limitation of it.

Every run stops and tells you exactly what to do whenever a human
decision is actually needed:

```
PAUSED for human approval. Review the change, then run:

    moor run my-app -- keel approve greet-function --stage spec

...and re-run this recipe to continue.
```

Content-authoring gates (spec/plan) get a bounded, tool-restricted
self-correction loop — a failing gate's own output is fed to an agent
that can only use `Write`/`Edit`, never Bash, so it can fix exactly what
the gate named without going and implementing the feature instead. The
actual build (`keel run`) gets no such auto-retry beyond a small attempt
cap — that step has full tool access, and iterating on it unattended is
exactly the scope creep this project's security posture argues against,
so it stops and hands the evidence to a human instead. See
[ADR-0005](docs/decisions/0005-recipe.md) for the full design, and
[docs/examples/recipe](docs/examples/recipe) for a real run's output —
including two real bugs this exercise found, with fixes.

Since a step like `keel run` can take a while and a recipe's own
progress narration used to only go to the terminal that launched it,
`moor logs <name> [--follow]` reads the same tamper-evident audit chain
and shows just the critical events — stage transitions, gate attempts,
pauses for approval — from any terminal, live:

```bash
moor logs my-app --follow
```

```
[17:31:07] [logs-verify] stage: spec
[17:31:07] [logs-verify] gate attempt 1/4: fail
[17:31:14] [logs-verify] gate attempt 2/4: fail
[17:31:25] [logs-verify] gate attempt 3/4: pass
[17:31:25] [logs-verify] stage: spec_approval
[17:31:25] [logs-verify] PAUSED — waiting on: spec_approval
```

See [ADR-0006](docs/decisions/0006-recipe-logs.md) — verified live,
watching a second terminal pick up real stage transitions and gate
failures within half a second of a real recipe run producing them.

## Layout

- `images/` — the sandbox base image + per-language layers (node, rust,
  python); see [docs/IMAGES.md](docs/IMAGES.md)
- `mcp/` — `moor-keel-mcp`, the MCP server that ships *inside* the
  sandbox and is the agent's only route to keel; see
  [docs/MCP.md](docs/MCP.md)
- `proxy/` — the egress gateway (default-deny forward proxy)
- `policies/` — the hardening baseline and the manifest schema, documented
- `cli/` — the `moor` Rust CLI, including `cli/templates/` (the
  per-project docker-compose template — lives inside the crate, not a
  top-level `compose/`, so a published crate can actually embed it)
- `docs/` — threat model, architecture, transport, images
- `Makefile` — local dev tooling (`make help`); mirrors
  `.github/workflows/ci.yml` so `make ci` runs the same checks locally

## Security

### Why you can trust this

Nothing here asks you to take moor's word for it — every claim below
is either checked by code you can read, or was verified live against a
real container and written up with the evidence attached:

- **The core guarantee is structural, not configurable.** No host bind
  mount, no `docker.sock`, `cap_drop: ALL`, non-root, read-only rootfs,
  a genuinely internal Docker network with no route out except through
  the egress gateway — see [docs/THREAT-MODEL.md](docs/THREAT-MODEL.md)
  for why each one is there. `moor selftest` re-checks all of it
  against the *live* container (not just the manifest) every time you
  run it, and fails safe: a corrupted or missing `docker inspect` field
  reads as FAIL, never as "all clear" (`missing_fields_default_to_failing_safe`
  in `cli/src/commands/selftest.rs`).
- **The audit trail is tamper-evident, not just append-only.** Every
  entry hash-chains to the one before it; `moor audit --verify`
  recomputes the whole chain and names exactly which entry was edited,
  deleted, reordered, or forged if any was — proven with a live tamper
  test in `tests/e2e.sh`, not just asserted. The chain is keel's
  `keel.chain/1` format, so keel's own `keel chain verify` checks it too,
  and it holds keel's gate verdicts and the sandbox's posture attestation
  alongside moor's own entries: one chain, not two logs to reconcile.
- **The agent's own permission checks are a second layer, not skipped.**
  Every project defaults to Claude Code's `auto` permission mode
  (`images/base/claude-settings.json`) instead of
  `--dangerously-skip-permissions` — verified directly against a live
  container: it creates files when asked and refuses a destructive
  `rm -rf` unprompted, with no human needed to answer a prompt. See
  [ADR-0003](docs/decisions/0003-claude-code-permissions.md). That
  means a compromised or manipulated agent run (the real risk category
  incidents like prompt-injection-driven tool abuse fall into) has to
  get past *two* independent things, not one: Claude's own per-call
  risk judgment, and the sandbox's structural containment below if that
  judgment is ever fooled.
- **The agent cannot advance its own work past a human checkpoint — and
  the first attempt at that claim was wrong, which is documented rather
  than quietly fixed.** The agent reaches keel through an MCP server moor
  ships into the sandbox ([docs/MCP.md](docs/MCP.md)), exposing `keel
  gate` and `keel next` and nothing else; `keel approve` is not a tool it
  defines, and every argv the server can produce is a compiled-in verb
  plus at most a validated slug and a gate id matched against a fixed
  table — asserted three ways in `mcp/src/main.rs`, including over every
  argv any accepted call can produce. But that alone did **not** hold:
  `--allowedTools` turned out to be an auto-approval list, not a
  restriction, so Claude Code ran `Bash` regardless and the `build` role
  reached `keel approve --help` through it. Every role now also passes
  `--disallowedTools` for every route to a shell or a subagent, which is
  what actually denies it — verified live both before and after. The
  honest accounting, including that this makes the control half list-based
  and therefore something moor must keep current, is in
  [ADR-0008's correction](docs/decisions/0008-agent-session-protocol.md).
- **An agent's reply cannot address your terminal.** The sandbox
  constrains what the agent can *execute*, not what it can *say* — and a
  reply drawn straight into a terminal can move the cursor, rewrite lines
  already read, or drive an OSC handler. Everything `moor studio` draws
  from a turn is stripped of every ANSI/CSI escape, every OSC sequence,
  and every C0 control except newline and tab (`\r` included) first. The
  renderer is a pure function, so what it *would* draw is asserted on
  directly in tests rather than eyeballed.
- **Every conversational turn is chained before you see it, and the chain
  holds no conversation.** `moor ask`/`moor studio` append one
  `agent-turn` entry — role, granted tool set, session id, SHA-256 hashes
  of prompt and response — before printing anything, so there is no code
  path to a response that skips the record. Text is deliberately *not* in
  the chain: `moor audit --export` bundles that file wholesale, so it goes
  to the redacted `transcript.jsonl` instead. Session continuity is the
  host's: the id is stored only after it validates as a UUID, and the
  agent's own response text is never parsed for one.
- **Every design decision that matters is written down, including the
  ones that didn't go moor's way.** ADR-0001 explains why gVisor and
  Apple's `container` tool were both rejected (and exactly what would
  need to change for that to flip); ADR-0002 explains why Claude Code
  auth is an injected token rather than a mounted `~/.claude`, including
  a proposal that *was* made and rejected during review. Nothing about
  the threat model is asserted without the reasoning attached.
- **CI enforces this on every push, and was itself verified for real** —
  not just written and assumed to work. [docs/CI.md](docs/CI.md) covers
  Rust lint/audit/deny, shellcheck, hadolint, gitleaks, a Trivy CVE scan
  of every built image, and the full `tests/e2e.sh` battery; every job
  was run against real GitHub Actions before being called done, and the
  doc names the real bugs that surfaced doing it (a broken Bash tool
  under the read-only rootfs, a silently-corrupted secret from a
  triple-paste, an end-of-life Node runtime) rather than just the tools'
  names.
- **The repo is public.** The threat model, every hardening control, and
  every known gap below are readable by anyone, including someone
  deciding whether to attack it — the design has to hold up without
  relying on obscurity, and that's a deliberate choice, not an oversight.

### Open concerns — not yet closed out

Being direct about what isn't solved is part of the trust case above,
not separate from it:

- **No branch protection on `master`.** CI reports pass/fail; nothing
  currently *blocks* a push that fails it. Low-stakes for a single-
  operator repo today, but worth fixing before this has other
  committers.
- **No scheduled re-scan.** Trivy/`cargo audit`/`cargo deny` only run
  when code changes (`push`/`pull_request` triggers) — a CVE published
  against an already-built, unchanged image or dependency tree goes
  undetected until the next commit, which could be a long time.
- **No vulnerability disclosure process.** Public repo, no `SECURITY.md`
  — there's no documented way for someone who finds a real issue to
  report it privately instead of opening a public issue.
- **The shared-kernel risk is open, not mitigated.** Staying on `runc`
  (ADR-0001) means a kernel/runc syscall escape isn't defended in depth
  the way gVisor or per-container VMs would; the only current mitigation
  is keeping Docker/OrbStack current. gVisor was rejected because this
  platform can't run it today, not because the risk it addresses isn't
  real.
- **Debian CVEs with no fix yet stay in every image, indefinitely.**
  `trivy --ignore-unfixed` is the right CI gate (see docs/CI.md for why),
  but it deliberately doesn't fail on `affected`/`fix_deferred`/
  `will_not_fix` findings — real, currently-unpatchable exposure that
  the scan will never turn red for.
- **Secret values are briefly visible in one subprocess's `argv`.**
  `security add-generic-password -w <value>` (macOS Keychain writes) has
  no non-interactive API that avoids this — documented as an accepted
  tradeoff in `cli/src/secrets.rs`, not something moor's own code
  can route around.
- **No image signing, provenance, or SBOM.** Nothing today lets you
  cryptographically verify a built image matches this source, or hand
  someone a machine-readable bill of materials for one.
- **Auto mode's risk judgment is Anthropic's to maintain, not moor's
  to verify per-release.** [ADR-0003](docs/decisions/0003-claude-code-permissions.md)
  replaced the blanket `--dangerously-skip-permissions` default with
  Claude Code's own `auto` permission mode — a real improvement, checked
  directly against a live container — but that mode's actual judgment
  calls are a model behavior moor doesn't control and hasn't
  re-verified across every future Claude Code version. An operator can
  still pass `--dangerously-skip-permissions` explicitly for a given run
  if they want the old behavior back.
- **No independent review.** Everything above is self-assessed — this
  project's own docs, ADRs, CI, and one real walkthrough. No third-party
  penetration test or external security audit has been done.

### Where this goes next

- A scheduled (nightly/weekly) CI run of the Trivy/audit/deny jobs,
  independent of code changes, to catch newly disclosed CVEs against
  images that haven't otherwise changed.
- Branch protection requiring CI to pass before merge, once this has
  more than one committer.
- A `SECURITY.md` with an actual disclosure contact/process.
- Image signing (cosign/sigstore) and SBOM generation (syft), published
  alongside each build.
- Revisit gVisor if a self-managed Linux host is ever justified; revisit
  Apple's `container` tool if
  [apple/container#719](https://github.com/apple/container/discussions/719)
  (the host-gateway egress leak) closes with a real fix.
- A custom seccomp profile derived from real `strace` output of an
  actual toolchain run, replacing the current default profile (already
  flagged as deferred in `policies/README.md`).
- A larger active breakout battery in `moor selftest` — today's
  three probes (canary domain, read-only fs, `docker.sock`) are a
  starting point, not a ceiling.
- Automated dependency updates (Dependabot/Renovate) for `Cargo.lock`
  and the toolchain versions baked into each image, instead of relying
  on someone remembering to bump them.
- Re-verify `auto` permission mode's actual behavior (does it still
  create files when asked, still refuse a destructive command
  unprompted) whenever the base image bumps its Claude Code version —
  it's a model behavior, not a pinned rule moor controls.

## Testing

- `cd cli && cargo test` — 122 unit tests: selftest's hardening evaluator
  (fed synthetic `docker inspect` JSON, including fail-safe-on-missing-data
  cases), manifest validation and round-tripping, compose template
  rendering, the tinyproxy access-log parser, the audit hash chain
  (append/verify, plus deliberately editing, deleting, reordering, and
  forging entries to confirm `--verify` catches each one; a golden entry
  written by keel that moor must hash identically; sealing a legacy
  chain; folding keel's sink, malformed lines included), the posture
  attestation derived from `docker inspect` (unproven when a field is
  missing), push ref and commit capture, Keychain
  service-name scoping, `moor import`'s image auto-detection and
  GitHub-URL parsing, `moor keel`/`moor view`'s project-resolution
  precedence (explicit flag, sticky `moor use` default, the one sandbox
  that's up, ambiguity errors), `moor keel`'s argv building, `moor use`'s
  known-project validation, `moor view`'s artifact-name mapping and
  markdown highlighting, the agent session protocol (role tool sets,
  `is_error`-based turn failure, UUID-validated session continuity, the
  content-free `agent-turn` chain entry, transcript redaction, recipe
  emission), and `moor studio` (one container query per refresh, input
  handled while a turn is in flight, escape-sequence stripping, the
  two-distinct-key approval, the `docker exec -i` artifact round-trip,
  per-project session isolation, stage read from `keel next`).
- `cargo test --manifest-path mcp/Cargo.toml` — 8 unit tests over the MCP
  server's tool surface: that it covers `gate` and `next` and that no argv
  any accepted call can produce reaches an advancement verb, that a slug
  must pass the same rule `manifest::validate_name` applies, that a gate
  id is matched against a fixed table rather than passed through, and that
  an unexpected argument key is refused rather than ignored.
- `./tests/e2e.sh` — end-to-end against real Docker containers: creates a
  throwaway project, runs `moor selftest`'s static checks and active
  breakout battery, confirms egress allow/deny against github.com and
  example.com, confirms a `git push` attempt is recorded with its ref,
  confirms the posture attestation is in the chain and that the agent
  cannot replace it, folds keel's sink and the egress log in, checks the
  host chain with `keel chain verify` (`$KEEL`, default `keel`), verifies
  the chain, **live-tampers with the real chain.jsonl file and confirms
  `--verify` detects it**, exports an audit bundle, and imports a
  throwaway local multi-branch repo end-to-end (image auto-detect, `keel
  init`, both branches present, selftest still passes). Requires the
  images to already be built (`./images/build.sh`).
- Both of the above, plus security/quality scanning (Rust lint/audit/deny,
  shellcheck, hadolint, gitleaks, Trivy image CVE scans), run in CI on
  every push/PR — see [docs/CI.md](docs/CI.md).
- `make help` lists every local dev target — `make check` for a fast
  fmt/clippy/test pass, `make security` for the full scan suite,
  `make ci` to run everything CI runs (including the real `tests/e2e.sh`)
  in one shot. Each security target checks its tool is installed and
  says how to get it (`brew install ...`) rather than failing on a bare
  "command not found." Every target here was run for real before being
  documented, not just written.

## Development

```bash
make help          # list every target
make check          # fast: fmt-check, clippy, cargo test — no Docker needed
make security        # cargo audit/deny, shellcheck, hadolint, gitleaks, trivy
make ci               # everything above plus the real tests/e2e.sh — the full CI replica
make build            # cargo build (debug)
make release          # cargo build --release
make install          # cargo install the CLI to ~/.cargo/bin
make clean            # cargo clean
```

`make publish-dry-run` runs `cargo publish --dry-run`. This crate was
originally named `isolator` and marked `publish = false` on purpose —
`isolator` is already taken on crates.io by an unrelated package, and
the crate wasn't meant to be published at all at the time. Three things
were needed to actually fix that, not just the first one that looked
like the whole story: renamed to `moor` (available, checked directly
against the crates.io API — see
[ADR-0004](docs/decisions/0004-rename-to-moor.md) for the naming
process and every alternative tried), MIT-licensed
(`LICENSE` + `license = "MIT"` in `cli/Cargo.toml`), and — found only by
actually running `cargo publish --dry-run --allow-dirty` all the way
through, not stopping at the first passing check —
`cli/src/compose.rs`'s `include_str!` moved from a top-level `compose/`
into `cli/templates/`, since a published crate can't embed a file that
lives outside its own package directory. `cargo publish --dry-run` now
packages, compiles from the isolated package tarball, and gets to the
upload step (aborted only because it's a dry run) — verified, not
assumed.

`make publish` runs the real thing. It's still a deliberate manual step
for whoever owns the crates.io account — nothing here runs it for
you — but it exists now: it re-runs `publish-dry-run` first, then
requires typing the literal word `publish` at a prompt (not just
pressing enter) before calling `cargo publish` itself, since a crates.io
version can be yanked afterward but never deleted or reused. Needs
`cargo login` done once beforehand, with a token from
https://crates.io/settings/tokens. One caveat ADR-0004 is explicit about:
`cargo install moor` only brings the compiled binary, not `images/`,
`proxy/`, or `policies/` — without those (from a full
clone, with `./images/build.sh` run once) the installed binary has no
sandbox images to build from.

## Status

Phases 0–9 of the plan are done, with Phase 9 partial by design (see
below):

- **0–3**: threat model/architecture docs, base + language images, the
  egress gateway with a default-deny allow-list, the container hardening
  baseline (read-only rootfs, dropped capabilities, no bind mounts,
  non-root user, no default-bridge network).
- **4**: the `moor` CLI (`new`, `up`, `down`, `shell`, `run`, `status`,
  `audit`, `selftest`).
- **5**: keel + the Claude Code CLI verified running inside the sandbox
  (`keel init`, `keel run` all execute there, not on the host).
- **6**: a hash-chained, tamper-evident audit trail per project
  (`audit/chain.jsonl`) folding in exec commands, egress verdicts, and a
  distinct `git-push` tag; `moor audit --verify` and `--export`.
- **7**: `moor selftest` now also runs an active breakout battery
  (canary-domain reachability, read-only-fs write attempt, `docker.sock`
  presence) against a live container, not just static `docker inspect`
  checks.
- **8**: [a real project run end-to-end](docs/WALKTHROUGH.md) — a private
  GitHub repo, a spec, two real gate-caught scope violations, a real
  `npm install`/`node --test`/`node --check` build inside the sandbox, a
  real `git push`, and a verified 51-entry audit trail. Found and fixed
  three real bugs: the private-repo clone had no credentials, the
  `--export` bundle pointed at a keel evidence path that doesn't exist,
  and (same root cause) the audit command's own help text repeated it.

- **9**: Keychain-sourced secrets are built (`moor secrets
  set`/`unset`/`status`; resolved automatically by `up`/`run`/`shell`,
  ahead of the operator's own shell env, with redaction wired through so
  a Keychain-only secret still gets scrubbed from the audit chain — this
  closed a real gap: it originally worked only within the single process
  that first resolved the secret, not later `moor run` invocations).
  Checked gVisor/runsc: not available under this OrbStack install (only
  `runc`), and switching to Docker Desktop wouldn't fix that either — its
  engine runs in the same kind of managed, non-administrable VM. Also
  looked at Apple's native `container` tool (its own-VM-per-container
  model would sidestep the "shared kernel" concern more fundamentally
  than gVisor does) — not usable on this Mac yet (needs a newer macOS
  than 15.3.1) and not a drop-in replacement for the `docker compose`
  layer this project is built on regardless. Both fully written up in
  `policies/README.md`'s Runtime rows.

  **Per-task ephemeral containers for `--waves` — deprioritized, not just
  deferred.** That item only matters if multiple keel tasks run
  concurrently in the same container, which only happens with `keel run
  --waves`. Daily usage here is serial (`gate g0` → `approve` → `gate g1`
  → `approve` → `run`, one stage at a time), where only one task is ever
  active in the sandbox at once — so there's no concurrent-task exposure
  to close today, and the canary token being per-project rather than
  per-task is a non-issue when there's only ever one task running. Revisit
  if `--waves` actually gets used.

  Still genuinely deferred: remote/centralized audit log shipping (no
  remote destination has been specified to ship to).

- **Migration**: [`moor import`](docs/MIGRATING.md) brings an
  existing local project into a sandbox — history transferred via a
  one-shot `git bundle` (never a bind mount), image auto-detected,
  existing GitHub remote preserved. Verified against keel's own repo:
  exact commit history, both branches, and its already-existing
  `.keel/keel.toml` correctly left alone instead of overwritten. Found
  and fixed two real bugs building it: `docker cp` silently fails against
  a read-only rootfs even onto a writable tmpfs mount (fixed by streaming
  through `docker exec`'s stdin instead), and cleaning up a stale
  bundle-path `origin` remote was deleting every non-default branch along
  with it (fixed by materializing branches locally first).

- **Recipe**: [`moor recipe`](docs/decisions/0005-recipe.md) drives
  keel's spec/gate/plan/gate/run pipeline from a loosely-described
  outcome, stopping for human approval at the same checkpoints keel
  already defines. Verified end-to-end on a real spec, gates failing and
  self-correcting for real (twice, on genuinely different problems each
  time) along the way — see [docs/examples/recipe](docs/examples/recipe).
  Found and fixed three real bugs building it: a concrete goal
  description reliably makes Claude Code implement the thing instead of
  writing a spec about it, fixed structurally with `--allowedTools`
  rather than prompt wording; `--allowedTools <tools...>` is variadic and
  swallows the prompt itself if the prompt comes after it; and keel's
  hard refusals (e.g. "already exists") print to stderr while gate
  results print to stdout, which `cli/src/proc.rs`'s stdout-only
  `run_capture` was silently dropping — fixed by adding
  `run_capture_combined` alongside it, not changing its existing callers.

- **Recipe logs**: [`moor logs`](docs/decisions/0006-recipe-logs.md)
  shows (or follows) a running recipe's stage transitions and gate
  attempts from any terminal, reading the same audit chain `moor audit`
  already writes rather than a second logging mechanism. Verified live:
  a `--follow` in one terminal picked up every stage transition and
  gate pass/fail from a real recipe run in another, in real time.

- **Git identity**: [`moor up`/`new`/`import`](docs/decisions/0007-git-identity.md)
  now sync the sandbox's git identity from the host's own `git config`
  — traced `keel approve`'s "unknown" straight to keel's own source
  (`git config user.name`, then `$USER`, then "unknown"; the sandbox had
  neither set). Verified live against the real `keel` project:
  `keel approve` went from recording "unknown" to recording "Dane
  Balia".
