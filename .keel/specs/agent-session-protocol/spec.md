---
id: SPEC-0003
slug: agent-session-protocol
schema: keel.spec/1
status: draft
scope:
  - "mcp/Cargo.toml"
  - "mcp/src/main.rs"
  - "images/base/Dockerfile"
  - "images/base/mcp-config.json"
  - "cli/src/session.rs"
  - "cli/src/commands/ask_cmd.rs"
  - "cli/src/commands/recipe.rs"
  - "cli/src/commands/mod.rs"
  - "cli/src/main.rs"
  - "cli/src/paths.rs"
  - "docs/decisions/0008-agent-session-protocol.md"
budget:
  criteria: 8
  lines: 500
verified_at: 2026-09-20
---

# A protocol for instructing the sandboxed agent and verifying its work

## Context

There are exactly two ways to put an instruction in front of the agent
today, and neither supports thinking a change through before committing
to it.

`moor shell` opens an interactive TTY and appends a single audit entry
whose entire redacted argv is the literal string `"shell"` — see
`cli/src/commands/shell.rs`. Everything said to the agent in that
session, and everything it did in reply, is outside the chain. `moor
recipe` is the opposite: fully automated from a recipe file written
blind, with no conversational step at all.

The transport is not the open question. `docs/ARCHITECTURE.md` already
states moor's pattern — it "shells out to `docker`, `docker compose` and
`gh` via `std::process::Command` — the same subprocess pattern keel uses
for its drivers (own process group, captured stdout/stderr, no shell
interpolation of untrusted strings)" — and `proc::run_with_stdin_file`
already pipes through `docker exec -i`. Both directions of this protocol
ride that, with no listening socket anywhere and nothing crossing a
network.

What is open is how the agent reaches keel. An earlier draft of this spec
granted a scoped shell (`--allowedTools "Bash(keel gate *)"`) and kept a
moor-maintained verb allowlist. That is the control ADR-0003 already
rejected: a hand-written allow/deny list "would need moor to track Claude
Code's tool surface indefinitely and would rot every time that surface
changes — exactly the kind of configuration-based control this project
has otherwise avoided in favor of structural ones." The rot was immediate
— checked against the image's own Claude Code 2.1.270, the syntax is
`Bash(git *)` with a space, not the colon form that draft assumed.

So the agent reaches keel through an MCP server instead. moor ships a
small server in the sandbox exposing the verification verbs as tools.
`approve` is not a tool it defines, so there is no allowlist entry to
maintain and no denial to enforce — the verb does not exist in the
agent's world. That is structural rather than configurable, which is the
distinction the README's Security section draws, and MCP is a protocol
Anthropic maintains rather than one moor has to track.

Three behaviours were verified directly against a live sandbox at Claude
Code 2.1.270, not assumed:

- `--output-format json` returns `session_id`, `result`, `is_error`,
  `num_turns` and `permission_denials`.
- A failed turn returned `"is_error": true` alongside `"subtype":
  "success"`. `subtype` is not a success signal.
- `--allowedTools` is variadic and swallows a trailing positional, so the
  prompt must precede it — the same trap `cli/src/commands/recipe.rs:126`
  already documents.

## Acceptance criteria

### AC-1 The agent's keel surface contains no advancement verb

WHEN the MCP server reports its tool list THE SYSTEM SHALL return tools
covering only `gate` and `next`, and SHALL define no tool that invokes
`keel approve`, so advancing a spec past a human checkpoint is absent
from the agent's tool surface rather than denied by configuration.

oracle: cmd `cargo test --manifest-path mcp/Cargo.toml tools::tests::surface_is_verification_only -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-2 No caller-supplied text reaches keel's argv unvalidated

WHEN the MCP server builds the argv for a tool call THE SYSTEM SHALL
include only a spec slug that passes the same validation
`Manifest::validate_name` applies, and SHALL pass no other caller-supplied
string through to the child process, so a tool argument cannot smuggle a
second verb or a shell fragment into the invocation.

oracle: cmd `cargo test --manifest-path mcp/Cargo.toml tools::tests::rejects_unvalidated_arguments -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-3 Brainstorm turns cannot modify the workspace

WHEN a turn is issued in the `brainstorm` role THE SYSTEM SHALL pass a
tool set limited to `Read`, `Glob` and `Grep`, so the agent can ground a
discussion in the real repository without being able to change it.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml session::tests::brainstorm_grants_no_write_tool -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-4 Session continuity is host-authoritative

WHEN a turn completes THE SYSTEM SHALL persist the returned session
identifier host-side under `~/.moor/projects/<name>/` only after it
validates as a UUID, and resume the next turn from that stored value
rather than from any identifier appearing in the agent's response text.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml session::tests::rejects_malformed_session_id -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-5 A failed turn is detected by is_error

WHEN a turn's JSON result carries `"is_error": true` THE SYSTEM SHALL
treat that turn as failed regardless of the process exit status or the
value of `subtype`, so a turn that reports `"subtype": "success"` while
erroring is not recorded as a success.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml session::tests::error_turn_is_detected_by_is_error -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-6 Every turn is chained before its response is shown

WHEN a turn returns THE SYSTEM SHALL append one `agent-turn` entry to the
project's existing hash chain carrying the role, the granted tool set,
the session identifier, and SHA-256 hashes of the prompt and response —
and no prompt or response text — before printing anything to the
operator.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml session::tests::turn_is_chained_without_content -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-7 Stored transcripts are redacted

WHEN a turn's full text is written to the project's transcript file THE
SYSTEM SHALL pass it through `audit::redact` with that project's declared
secret names, so a value the agent echoed back is masked at rest the same
way `log_exec` already masks argv.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml session::tests::transcript_is_redacted -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-8 An emitted recipe is written by the host and parses

WHEN the operator ends a brainstorm by emitting a recipe THE SYSTEM SHALL
write that file on the host after it parses under the same parser
`moor recipe` uses, so the agent contributes text but never creates the
file that drives its own next run.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml session::tests::emitted_recipe_round_trips -- --exact 2>&1 | grep -q '1 passed'` exit 0

## Out of scope

_Any user interface._ This spec defines the protocol and its CLI surface
only. Driving it from a multi-project interface is spec
`studio-multi-project-console`, which depends on this one.

_Streaming._ Turns are request/response via `--output-format json`.
Incremental delivery via `--input-format stream-json`, and the
`run_capture_combined` buffering that currently hides `keel run` output
until it finishes, are a distinct concern with a distinct failure mode.

_Mutual exclusion between a turn and a running recipe._ Deferred, not
dismissed: both can write one workspace and nothing currently serialises
them. It needs a lock with a defined staleness rule, which is its own
change.

_Pinning the Claude Code version._ `images/base/Dockerfile:51` installs
`@anthropic-ai/claude-code` unpinned, so the JSON shape AC-5 keys on can
move without warning. Real, and worth its own spec; it is an image
concern, not a protocol one.

_Raising the change budget._ Set to 500 lines here deliberately — a new
crate, an image change and a session module do not fit the 120 this
repository's earlier specs used, and pretending otherwise would make the
G2 diff check meaningless rather than useful.
