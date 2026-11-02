---
id: SPEC-0004
slug: studio-multi-project-console
schema: keel.spec/1
status: draft
scope:
  - "cli/src/studio/mod.rs"
  - "cli/src/studio/state.rs"
  - "cli/src/studio/render.rs"
  - "cli/src/commands/studio_cmd.rs"
  - "cli/src/session.rs"
  - "cli/src/commands/mod.rs"
  - "cli/src/main.rs"
  - "cli/Cargo.toml"
budget:
  criteria: 8
  lines: 600
verified_at: 2026-09-20
---

# A multi-project console over the agent session protocol

## Context

Spec `agent-session-protocol` gives one project a conversational turn, a
keel tool surface for the agent, and a chained record of both. It is
driven one command at a time against one project named on the command
line. Everything an operator actually does spans more than that: which
projects exist, which are up, what stage each spec is at, what the last
gate said, and whether the plan is worth approving.

That information already exists and is already cheap to get.
`paths::all_project_names` enumerates projects from disk, and
`commands::running_projects` establishes live container state for every
project in a single `docker ps` rather than one call each. `keel next
--json` reports a spec's stage and next command in the shape
`cli/src/commands/recipe.rs` already parses. None of it is presented
anywhere at once.

This console is a terminal UI, consistent with how the rest of moor
presents things — `cli/src/commands/view.rs` hand-rolls ANSI highlighting
for spec, plan and tasks rather than take a markdown rendering
dependency for three known-simple file shapes. It runs locally against
the same `docker exec` subprocess path everything else uses. There is no
socket, no daemon and no network hop.

Two properties are load-bearing and easy to get wrong.

The first is that agent output is untrusted text being drawn into the
operator's terminal. The sandbox protects the host from the agent's
*execution*; a console that renders the agent's *bytes* reintroduces a
path the sandbox does not cover, because terminal escape sequences can
move the cursor, rewrite earlier lines, or drive OSC handlers.

The second is that `keel approve` is the human checkpoint the whole
pipeline is built around. `agent-session-protocol` removes it from the
agent's tool surface. A console that offers approval as a convenience —
bound to a repeated keypress, or applied to whichever spec happens to be
selected — puts it back in reach of a mistake instead of an agent, which
is the same checkpoint lost by a different route.

## Acceptance criteria

### AC-1 Live project state comes from one container query

WHEN the console refreshes its project list THE SYSTEM SHALL determine
which sandboxes are up from a single `docker ps` invocation covering
every project, so the refresh cost does not grow with the number of
projects on disk.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml studio::state::tests::refresh_issues_one_container_query -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-2 A turn in flight does not stall input handling

WHEN a turn has been sent and no response has returned THE SYSTEM SHALL
continue to accept keypresses and SHALL mark that project as awaiting a
response, so the console does not block on the child process.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml studio::tests::input_is_handled_while_turn_is_pending -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-3 Agent text is stripped of control sequences before it is drawn

WHEN the console renders text returned by a turn THE SYSTEM SHALL remove
every ANSI escape, OSC sequence and C0 control character except newline
and tab, so returned bytes cannot address the terminal directly.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml studio::render::tests::render_strips_control_sequences -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-4 Approval names its target and requires a distinct confirmation

WHEN the operator triggers a stage approval THE SYSTEM SHALL display the
project and spec slug it is about to approve and SHALL issue the verb
only after a second, different keypress confirms it, so no single
keystroke can approve a stage.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml studio::state::tests::approval_requires_a_second_distinct_key -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-5 Artifact edits round-trip through the existing exec path

WHEN the operator edits a spec, plan or tasks artifact THE SYSTEM SHALL
read it out of the container, open it in `$EDITOR` on the host, and write
it back via the same `docker exec -i` path `proc::run_with_stdin_file`
uses, so editing introduces no host bind mount.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml studio::tests::artifact_edit_writes_back_over_exec -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-6 Sessions do not cross projects

WHEN the operator selects a different project THE SYSTEM SHALL resume
that project's own stored session identifier and SHALL send no part of
the previous project's conversation, so context from one repository
cannot reach the agent working on another.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml studio::state::tests::switching_projects_isolates_sessions -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-7 Stage state is read from keel, not inferred

WHEN the console displays a spec's stage THE SYSTEM SHALL take it from
that project's `keel next --json` output, so displayed stage and gate
state cannot disagree with what keel itself would report.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml studio::state::tests::stage_is_read_from_keel_next -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-8 The console opens no second record

WHEN the console issues a turn or a keel verb THE SYSTEM SHALL record it
through the same chained-audit path those commands already use and SHALL
write no separate console-only log, so the chain remains the single
account of what happened in a project.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml studio::tests::console_writes_no_second_log -- --exact 2>&1 | grep -q '1 passed'` exit 0

## Out of scope

_Streaming responses._ Inherited from `agent-session-protocol`: turns are
request/response. The console shows a pending marker and the completed
reply, and gains incremental output only when that spec's deferred
streaming work lands.

_Remote or browser access._ The console is a local terminal process
driving local `docker exec`. No socket is opened and no port is bound.
Reaching this from another machine is a different design with an
authentication problem this one does not have.

_Running `keel run` from the console._ Approval and verification are
operator decisions that take a keystroke; a build is a long job whose
output belongs in `moor recipe` and `moor logs`, which already handle it.

_Editing inside the console._ `$EDITOR` is the editor. Building a text
editor into a project console is a larger commitment than the whole
feature is worth.

_Creating or destroying projects._ `moor new`, `moor import` and `moor
down` stay command-line operations. A console that can delete a project
by keypress is a worse tradeoff than one that cannot.
