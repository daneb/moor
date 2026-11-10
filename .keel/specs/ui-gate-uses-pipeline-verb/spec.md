---
id: SPEC-0026
slug: ui-gate-uses-pipeline-verb
schema: keel.spec/1
status: approved
scope:
- cli/src/ui/api.rs
budget:
  criteria: 3
  lines: 90
verified_at: 2026-11-10
---

# The console's gate action runs through moor pipeline, not the removed keel subcommand

## Context

The web console builds a fixed `moor` argv per action in
`start_job` (`cli/src/ui/api.rs`) and runs it as `moor <argv>`. The `gate`
action still builds its argv starting with `"keel"` — the pre-0.10.0 name
of the sandbox escape hatch. moor 0.10.0 renamed that subcommand to
`pipeline` (`Command::Pipeline`), so `moor keel -p <p> gate g1 <slug>`
now exits with `error: unrecognized subcommand 'keel'`. The action's
display label already reads `moor pipeline gate <gate> <slug>`, so the
label and the argv disagree; the label is right and the argv is stale.

Every other handler in `start_job` uses a real moor subcommand as the
first argv element (`go`, `spec`, `recipe`, `new`, `import`, `audit`).
Only `gate` was missed by the 0.10.0 rename. The fix is to build the gate
argv with `pipeline` first, so it dispatches to `Command::Pipeline`, which
prepends `keel` inside the sandbox.

## Acceptance criteria

### AC-1 the gate action's argv starts with the pipeline subcommand

WHEN `start_job` builds the argv for a `gate` action THE SYSTEM SHALL make
the first argv element the string `pipeline` and SHALL NOT make it the
string `keel`.

oracle: test ui::api::tests::gate_argv_dispatches_through_pipeline

### AC-2 the gate argv still carries project, gate and slug in order

WHEN `start_job` builds the argv for a `gate` action on gate `g1` and slug
`s` under project `p` THE SYSTEM SHALL produce the argv `pipeline -p p gate
g1 s`.

oracle: test ui::api::tests::gate_argv_dispatches_through_pipeline

### AC-3 the displayed label names moor pipeline

WHEN `start_job` builds a `gate` action THE SYSTEM SHALL set its label to
text beginning `moor pipeline gate`.

oracle: test ui::api::tests::gate_label_names_moor_pipeline

## Out of scope

- Any other action handler in `start_job` (all already use a real moor
  subcommand).
- The `Command::Pipeline` parsing or `pipeline_cmd` behaviour, which is
  correct and unchanged.
- The `g0`/`g1`-only restriction on which gates the console runs.
