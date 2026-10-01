---
id: SPEC-0013
slug: studio-decisions
schema: keel.spec/1
status: draft
scope:
  - "cli/src/studio/state.rs"
  - "cli/src/studio/render.rs"
  - "cli/src/studio/mod.rs"
budget:
  criteria: 8
  lines: 400
verified_at: 2026-10-01
---

# Decide and unblock from studio without dropping to the CLI

## Context

`moor studio` can already show every project's stage, hold a turn with the
agent, edit an artefact, and **approve** what a spec is waiting on — the
approval being an armed two-key confirmation in a terminal, which is what
makes it a human act rather than something a process can perform.

Two gaps force the operator back to the CLI mid-flow, which is why studio
is not yet the single interface to a project:

- **No rejection.** `a` approves; there is no way to reject. Rejection
  needs a reason (keel records a note), so it needs text entry, not just a
  keypress — studio already has a text input for the agent prompt, so the
  mechanism exists.
- **No gate output in context.** When a spec sits at a failing gate,
  studio shows the stage but not *which checks failed*. The operator sees
  "fix plan" with no idea what to fix, and leaves to run the gate by hand.
  The failing checks are already on disk in the sandbox
  (`.keel/specs/<slug>/gates/G*.json`), written by keel.

Closing both keeps the decision — approve or reject, with a reason — a
keypress in a terminal the operator owns, rather than a command any
process could run.

## Acceptance criteria

### AC-1 A key begins a rejection and takes a reason

WHEN the operator presses the reject key on a project whose spec is
waiting on a decision THE SYSTEM SHALL enter a state that collects a typed
reason before anything is recorded.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml reject_key_collects_a_reason` exit 0

### AC-2 An empty reason records nothing

IF the operator confirms a rejection whose reason is empty THEN THE SYSTEM
SHALL record no rejection and SHALL say a reason is required.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml empty_reason_records_no_rejection` exit 0

### AC-3 Rejection is confirmed, not a single keypress

WHEN a rejection has a reason THE SYSTEM SHALL require a second, distinct
confirming key before it is recorded, as approval already does.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml rejection_needs_a_second_key` exit 0

### AC-4 Escape abandons a rejection

WHEN the operator presses escape while entering a rejection THE SYSTEM
SHALL abandon it and record nothing.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml escape_abandons_a_rejection` exit 0

### AC-5 The recorded rejection carries keel's own command and the reason

WHEN a rejection is confirmed THE SYSTEM SHALL emit keel's own reject
command for that stage with the typed reason as its note, rather than a
command reassembled from the stage name.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml rejection_uses_keels_own_command_and_note` exit 0

### AC-6 A failing gate's checks are shown with the project

WHEN a project's spec is at a gate that last failed THE SYSTEM SHALL show
the names of the failing checks alongside that project.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml failing_checks_are_shown` exit 0

### AC-7 Gate output is sanitized before it is drawn

WHEN gate output is drawn THE SYSTEM SHALL strip terminal control
sequences from it first, as every other sandbox-sourced string already is.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml gate_output_is_sanitized` exit 0

### AC-8 Existing CLI tests stay green

THE SYSTEM SHALL leave the existing CLI test suite passing.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml` exit 0

## Out of scope

- Turning a conversation into a spec or a plan — sibling spec
  `studio-authoring`.
- Showing the full diff at a merge decision (the CLI's `moor approve`
  does this); studio shows the failing checks of a *gate*, which is the
  thing that currently has no visibility at all.
- Running a gate from studio: the stage already tells the operator the one
  command, and `moor go` runs it. This spec is about *seeing why it
  failed* and *recording a decision*, not about adding a runner.
