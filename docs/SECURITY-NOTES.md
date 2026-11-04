# Security: the case, the gaps, and what's next

Moved out of the README. The short version is in the README; this is the detail.

## Why you can trust this

Nothing here asks you to take moor's word for it — every claim below
is either checked by code you can read, or was verified live against a
real container and written up with the evidence attached:

- **The core guarantee is structural, not configurable.** No host bind
  mount, no `docker.sock`, `cap_drop: ALL`, non-root, read-only rootfs,
  a genuinely internal Docker network with no route out except through
  the egress gateway — see [docs/THREAT-MODEL.md](THREAT-MODEL.md)
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
  [ADR-0003](decisions/0003-claude-code-permissions.md). That
  means a compromised or manipulated agent run (the real risk category
  incidents like prompt-injection-driven tool abuse fall into) has to
  get past *two* independent things, not one: Claude's own per-call
  risk judgment, and the sandbox's structural containment below if that
  judgment is ever fooled.
- **The agent cannot advance its own work past a human checkpoint — and
  the first attempt at that claim was wrong, which is documented rather
  than quietly fixed.** The agent reaches keel through an MCP server moor
  ships into the sandbox ([docs/MCP.md](MCP.md)), exposing `keel
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
  [ADR-0008's correction](decisions/0008-agent-session-protocol.md).
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
  not just written and assumed to work. [docs/CI.md](CI.md) covers
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

## Open concerns — not yet closed out

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
  to verify per-release.** [ADR-0003](decisions/0003-claude-code-permissions.md)
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

## Where this goes next

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
