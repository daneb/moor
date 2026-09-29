---
id: SPEC-0008
slug: tool-call-risk-gate
schema: keel.spec/1
status: approved
scope:
- docs/adr/0001-tool-call-risk-gating.md
- cli/src/decision.rs
- cli/src/commands/run_cmd.rs
- cli/src/audit.rs
- cli/src/main.rs
- docs/THREAT-MODEL.md
budget:
  criteria: 8
  lines: 350
verified_at: 2026-09-27
---

# Tool-call risk gating: ADR and rules-only plumbing

## Context

A spike proposes flagging risky agent tool calls with a local decision
model, gated by a "PreToolUse"-style hook. `docs/THREAT-MODEL.md` already
rejected that shape of control for git pushes: a hook that runs inside the
sandbox is reconfigurable by the same agent it is meant to constrain (see
"Why not a git pre-push hook"). The same reasoning applies to any
in-sandbox agent hook. `moor run` (`cli/src/commands/run_cmd.rs`) is the
one place a command from `moor run <project> -- <cmd>` crosses from the
host into `docker exec` today, and today it does not gate anything — it
execs unconditionally and only audits afterward. This spec puts a
deterministic, rules-only gate at that host-side point, and records the
verdict in the ADR that a model backend (no model exists yet) would later
plug into, without building the model, daemon, or "ask" workflow yet.

## Acceptance criteria

### AC-1 The ADR names the enforcement point

THE SYSTEM SHALL ship `docs/adr/0001-tool-call-risk-gating.md` stating that
risk verdicts are enforced host-side in `moor run` before `docker exec`
runs, and that an in-sandbox agent hook is a signal only, never the
enforcement point.

oracle: cmd `test -f docs/adr/0001-tool-call-risk-gating.md && grep -q 'docker exec' docs/adr/0001-tool-call-risk-gating.md && grep -qi 'in-sandbox' docs/adr/0001-tool-call-risk-gating.md` exit 0

### AC-2 Credential-path commands are denied

WHEN a `moor run` command argument matches a known credential path
(`.ssh`, `.aws`, `.netrc`, `id_rsa`, or `.pem`) THE SYSTEM SHALL return a
`Deny` verdict with rule id `credential_access`.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml decision::tests::denies_known_credential_paths -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-3 Docker-socket and privilege-escalation commands are denied

WHEN a `moor run` command references the Docker socket path or invokes
`sudo` THE SYSTEM SHALL return a `Deny` verdict with rule id
`boundary_escape`.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml decision::tests::denies_docker_socket_and_sudo -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-4 Whole-workspace or home-directory wipes are denied

WHEN a `moor run` command is `rm` with a recursive-force flag targeting
`/`, `/workspace`, or `~` THE SYSTEM SHALL return a `Deny` verdict with
rule id `destructive`.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml decision::tests::denies_recursive_wipe_of_workspace_or_home -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-5 A Deny verdict blocks execution before it reaches Docker

IF `decision::evaluate` returns `Deny` THEN THE SYSTEM SHALL append a
`decision` entry to the project's audit chain recording the rule id and
the redacted command, and `moor run` SHALL exit non-zero without invoking
`docker exec`.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::deny_verdict_blocks_docker_exec -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-6 An Allow verdict changes nothing observable

WHEN `decision::evaluate` returns `Allow` THE SYSTEM SHALL invoke
`docker exec` and append the `exec`/`push` audit entry exactly as `moor
run` did before this change.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet` exit 0

## Out of scope

- The local decision model, ONNX export, and the `ort`-backed daemon —
  `decision::evaluate` is a fixed Rust rule table, not a model backend.
- An "ask" verdict and any human-in-the-loop UI for it; v1 only decides
  `Allow`/`Deny`.
- Any in-sandbox hook (e.g. a Claude Code `PreToolUse` hook) as a source
  of *signal* — deferred to a later spec once the host-side gate above is
  in place.
- Commands typed inside an interactive `moor shell` session, or MCP-tool
  calls dispatched inside the sandbox — neither passes through `moor run`.
- Dataset collection, teacher labelling, training, and calibration.
- Any agent other than Claude Code, and any command other than what
  `moor run` executes.
