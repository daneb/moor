# Project history

How moor got here, phase by phase. The README no longer carries this.

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
- **8**: [a real project run end-to-end](WALKTHROUGH.md) — a private
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

- **Migration**: [`moor import`](MIGRATING.md) brings an
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

- **Recipe**: [`moor recipe`](decisions/0005-recipe.md) drives
  keel's spec/gate/plan/gate/run pipeline from a loosely-described
  outcome, stopping for human approval at the same checkpoints keel
  already defines. Verified end-to-end on a real spec, gates failing and
  self-correcting for real (twice, on genuinely different problems each
  time) along the way — see [docs/examples/recipe](examples/recipe).
  Found and fixed three real bugs building it: a concrete goal
  description reliably makes Claude Code implement the thing instead of
  writing a spec about it, fixed structurally with `--allowedTools`
  rather than prompt wording; `--allowedTools <tools...>` is variadic and
  swallows the prompt itself if the prompt comes after it; and keel's
  hard refusals (e.g. "already exists") print to stderr while gate
  results print to stdout, which `cli/src/proc.rs`'s stdout-only
  `run_capture` was silently dropping — fixed by adding
  `run_capture_combined` alongside it, not changing its existing callers.

- **Recipe logs**: [`moor logs`](decisions/0006-recipe-logs.md)
  shows (or follows) a running recipe's stage transitions and gate
  attempts from any terminal, reading the same audit chain `moor audit`
  already writes rather than a second logging mechanism. Verified live:
  a `--follow` in one terminal picked up every stage transition and
  gate pass/fail from a real recipe run in another, in real time.

- **Git identity**: [`moor up`/`new`/`import`](decisions/0007-git-identity.md)
  now sync the sandbox's git identity from the host's own `git config`
  — traced `keel approve`'s "unknown" straight to keel's own source
  (`git config user.name`, then `$USER`, then "unknown"; the sandbox had
  neither set). Verified live against the real `keel` project:
  `keel approve` went from recording "unknown" to recording "Dane
  Balia".
