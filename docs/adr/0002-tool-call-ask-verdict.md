# ADR-0002: Ask is an interactive operator confirm, fail-closed when unattended

## Status

Accepted. Implemented by keel spec `tool-call-ask-verdict`.

## Context

ADR-0001 made every `decision::evaluate` rule match a hard `Deny`: no
override, by design, because nothing sat on the other end of an
in-sandbox agent to ask. But `moor run <project> -- <cmd>` and `moor keel
<verb>` are typed by the operator at their own terminal — the same human
`keel approve` already trusts with every checkpoint in this pipeline. A
false positive (a `.pem` file that happens to be a public cert, not a
private key) had no way through but editing the rule table by hand.

The spike's `Ask` verdict is "a human decides." For `moor run`'s actual
callers, the human is already present when the command is typed — this
does not need a daemon, a socket, or an approval queue to be real.

While scoping this, `cli/src/commands/recipe.rs`'s unattended agent-step
execution turned out to call Docker directly, not through
`run_cmd::gate` — so it isn't covered by ADR-0001's gate at all today,
let alone by this one. Left open deliberately (see "Deferred" in
ADR-0001 and this ADR's own scope): an unattended flow has no terminal to
prompt, so closing that gap is an async-decision or model problem, not
an extension of the confirm this ADR adds.

## Decision

A rule match calls `confirm`, which prints the rule id and reason and
reads one line from stdin, but only when `moor run`'s stdin is a real
terminal (`std::io::IsTerminal`). Only `y`/`yes` (trimmed,
case-insensitive) is a yes; anything else — a decline, empty input, a
read error, or a non-interactive invocation that never reads stdin at
all — denies exactly as ADR-0001 already did. Every resolution, allowed
or denied, is logged to the audit chain (`audit::log_decision`) with the
rule id, the resolution, and the operator's identity from `git config
user.name` (`unknown` when unset) — the same fallback keel's own approval
records use.

## Consequences

- An operator can now get past a false-positive rule match without
  editing `decision.rs`, at the cost of one explicit, logged confirm.
- Nothing about a scripted or headless `moor run` changes: no prompt is
  ever shown, and stdin content is never consulted, when stdin isn't a
  terminal. `moor recipe` and keel's own driver are exactly as
  fail-closed as before this ADR.
- A rule match still denies by default in every case where no one
  answers `y` — declining, timing out at an empty line, or running
  unattended all resolve the same way.

## Deferred

- `moor recipe`'s unattended agent steps, which don't pass through this
  gate at all yet.
- Remembering a decision across invocations; every match prompts again.
- The local decision model, ONNX, and the `ort`-backed daemon — this ADR
  changes nothing about what triggers a match, only what happens once one
  does.
