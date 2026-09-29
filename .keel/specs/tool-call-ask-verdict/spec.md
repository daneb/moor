---
id: SPEC-0009
slug: tool-call-ask-verdict
schema: keel.spec/1
status: approved
scope:
- docs/adr/0002-tool-call-ask-verdict.md
- cli/src/commands/run_cmd.rs
- cli/src/audit.rs
budget:
  criteria: 8
  lines: 300
verified_at: 2026-09-28
---

# Tool-call risk gate: interactive Ask verdict

## Context

SPEC-0008 (ADR-0001) made every rule match in `decision::evaluate` a hard
`Deny`: no override, ever. That is correct for a fully unattended path,
but `moor run`/`moor keel` are typed by the operator at their own
terminal — the one place moor's threat model already trusts a human
decision (`keel approve`, "the human checkpoints keel defines"). A false
positive today (a `.pem` file that is a public cert, not a private key)
has no escape hatch but editing the rule table.

This spec turns that hard `Deny` into the spike's `Ask`: prompt the
operator, with the rule id and reason, only when `moor run`'s stdin is a
real terminal. Any other input, or no input at all, denies exactly as
before — the fail-closed behavior automated and headless invocations
already depend on is unchanged. Every prompted decision, allowed or
declined, is logged to the audit chain with its resolution and the
operator's git identity, so an override is never silent.

## Acceptance criteria

### AC-1 An interactive operator can override a flagged command

WHEN `moor run`'s stdin is a terminal and a command matches a rule THE
SYSTEM SHALL print the rule id and reason and prompt the operator, then
proceed to `docker exec` only if the operator answers `y` or `yes`
(case-insensitive).

oracle: cmd `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::interactive_yes_allows -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-2 A decline denies exactly as an unprompted Deny does

WHEN the operator answers anything other than `y`/`yes`, including empty
input, THE SYSTEM SHALL deny the command with no `docker exec` invoked.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::interactive_no_denies -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-3 A non-interactive invocation is never prompted

IF `moor run`'s stdin is not a terminal THEN THE SYSTEM SHALL deny a
flagged command without reading stdin or printing a prompt, regardless of
what stdin contains.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::non_interactive_denies_without_reading_stdin -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-4 Every prompted decision is attributed in the audit chain

WHEN a rule match is resolved, by prompt or by fail-closed denial, THE
SYSTEM SHALL append a `decision` audit entry recording `rule_id`,
`resolution` (`allow` or `deny`), and the operator identity from `git
config user.name` (`unknown` when unset).

oracle: cmd `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::decision_entry_records_resolution_and_operator -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-5 An Allow verdict is still a no-op

WHEN `decision::evaluate` returns `Allow` THE SYSTEM SHALL run the
command exactly as before this change: no prompt, no new audit entry
kind, unchanged existing behavior.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet` exit 0

## Out of scope

- `moor recipe`'s unattended agent-step execution, which calls Docker
  directly and does not pass through `run_cmd::gate` at all — discovered
  while scoping this spec, and left open deliberately: an unattended flow
  cannot show a TTY prompt, so closing that gap needs either an async
  decision path or the model, not this interactive confirm.
- Any change to `decision::evaluate`'s rule table (credential_access,
  boundary_escape, destructive) — same three rules, now with an override
  path instead of none.
- The local decision model, ONNX, and the `ort`-backed daemon.
- Remembering a decision across invocations (e.g. "always allow this
  rule for this project") — every match prompts again.
