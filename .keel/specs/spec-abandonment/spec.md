---
id: SPEC-0019
slug: spec-abandonment
schema: keel.spec/1
status: approved
scope:
- cli/src/commands/flow_cmd.rs
- cli/src/guide.rs
- cli/src/commands/mod.rs
- cli/src/main.rs
budget:
  criteria: 8
  lines: 160
verified_at: 2026-11-02
---

# Abandon a spec as a terminal state

## Context

The pipeline models exactly one terminal state: `Complete` (keel
`src/pipeline.rs`, `enum Stage`). Every spec that is not shipped is "open" —
`guide::pick_active` filters on `!complete`, so an open spec surfaces in
`moor next` forever. `moor reject` (`cli/src/commands/flow_cmd.rs`) does not
retire a spec; it records a rejection on a gate, which sends the spec *back*
for revision (`Standing::Rejected`/`Superseded` + a `recheck`). There is no
verb that means "we are not doing this spec."

The result is dead specs. On 2026-11-02 four specs that passed G0 but were
never approved (`compose-project-namespace`, `document-host-deployed-projects`,
`agent-session-protocol`, `studio-multi-project-console`) were moved by hand to
`.keel/specs/_abandoned/` just to clear the active view. That stopgap relies on
`keel spec list` only enumerating direct child folders of `.keel/specs/` (keel
`src/spec/mod.rs`, `list`), and it loses the audit trail of *why* and *who*.

keel owns spec status; moor drives it over the sandbox boundary. keel's
`SpecFront` already carries a `status` field (`src/spec/mod.rs`, defaulting
`draft`) that `derive_stage` ignores today — the natural insertion point for an
`abandoned` terminal state. This spec covers moor's host-side surface:
a guided `moor drop` verb, its guidance, and the command wiring. The keel-side
state (`Stage::Abandoned`, a `keel spec abandon` recorder, and filtering it from
`keel next --json`) is a dependency tracked in the keel repo; moor's criteria
that depend on it are verified by a human against a keel build that has it.

## Acceptance criteria

### AC-1 A drop verb records abandonment with a reason

WHEN the operator runs `moor drop "<why>"` with a non-empty reason THE SYSTEM
SHALL record the active spec as abandoned via keel and print the updated
guidance.

oracle: test cli/src/commands/flow_cmd.rs::drop_records_abandonment_with_a_reason

### AC-2 An empty reason is refused

IF the operator runs `moor drop` with a blank or whitespace-only reason THEN
THE SYSTEM SHALL refuse, record nothing, and exit non-zero.

oracle: test cli/src/commands/flow_cmd.rs::drop_refuses_an_empty_reason

### AC-3 Drop is distinct from reject at the boundary

THE SYSTEM SHALL send keel the abandon command for `moor drop` and the reject
command for `moor reject`, so a drop can never be recorded as a gate rejection
nor a rejection as an abandonment.

oracle: test cli/src/commands/flow_cmd.rs::drop_and_reject_send_distinct_keel_commands

### AC-4 An abandoned spec is not shown as active

WHERE a spec is abandoned THE SYSTEM SHALL exclude it from the spec
`guide::pick_active` returns, so `moor next` never points the operator at it.

oracle: test cli/src/guide.rs::an_abandoned_spec_is_never_the_active_spec

### AC-5 An abandoned spec reports a terminal step label

WHEN `guide::step_label` is asked about an abandoned spec THE SYSTEM SHALL
return the label `abandoned` rather than a step number.

oracle: test cli/src/guide.rs::step_label_of_an_abandoned_spec_is_abandoned

### AC-6 Dropping at any pre-merge stage leaves no guided dead-end

WHILE a spec is at any stage before `merge_approval` THE SYSTEM SHALL accept
`moor drop` and move the spec to the abandoned state without requiring a
`--force` flag from the operator.

oracle: test cli/src/commands/flow_cmd.rs::drop_is_accepted_at_every_pre_merge_stage

### AC-7 keel exposes an abandoned terminal state

THE SYSTEM SHALL treat a spec whose keel status is `abandoned` as terminal in
`keel next --json`, emitting it with `complete` true or omitting it from the
open set, so moor's host-side filter has an authoritative signal to read.

oracle: human reviewer runs `keel next --json` against a keel build carrying the
abandoned state on a spec marked abandoned and confirms it is not in the open set

### AC-8 Abandonment is recorded with who and why

THE SYSTEM SHALL persist each abandonment with the operator identity and the
reason, in the same audited location keel writes approvals, so the decision is
auditable and reversible.

oracle: human reviewer confirms an abandon record carries `by` and `note` and
appears in the spec's audit trail alongside approvals

## Out of scope

- Deleting or garbage-collecting abandoned spec folders. Abandonment is a
  state change, not a delete; reviving a spec stays possible.
- A bulk "abandon every stalled draft" command. This change adds the single
  per-spec verb; batch retirement can build on it later.
- Any change to what `moor reject` means. Reject still sends a spec back for
  revision; drop is the new, separate terminal path.
- Migrating the four specs already hand-moved to `.keel/specs/_abandoned/`.
