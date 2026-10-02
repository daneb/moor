---
id: SPEC-0012
slug: agent-session-abstraction
schema: keel.spec/1
status: approved
scope:
- cli/src/session.rs
- cli/src/manifest.rs
budget:
  criteria: 9
  lines: 480
verified_at: 2026-10-01
---

# Drive moor's own agent turns through any configured agent, not just Claude

## Context

moor has two independent routes to an agent, and only one of them honours
the project's chosen agent:

- **keel's driver** (`keel run --driver <id>`) already supports
  claude-code, copilot, kiro and codex, and SPEC-0010 made the agent
  selectable at `import`/`new`.
- **moor's own turns** (`moor ask`, `moor studio`, `moor recipe`'s
  spec-authoring step) go through `session.rs`, which is hardcoded to
  Claude: `build_turn_argv` emits `claude --print --output-format json
  …`, `parse_turn` expects Claude's JSON envelope (`is_error`,
  `session_id`), `next_session_id` validates Claude's UUID for
  `--resume`, and `failure_hint` translates Claude's "Not logged in".

So a project created with `--agent copilot` builds with Copilot but
*converses* with Claude — and on a sandbox with no Claude credential,
`moor ask`/`studio` fail outright. This blocks `moor studio` from being
the single interface to a project, because the agent half of it only
speaks to one agent.

**A safety asymmetry makes this more than plumbing.** Role-based tool
restriction is a real control: `Role` grants a narrow tool set, and
[ADR-0008](../../docs/decisions/0008-agent-session-protocol.md) records
that `--allowedTools` is an *auto-approval* list, not a restriction —
Claude ran `Bash` regardless — and that `--disallowedTools` is what
actually denies. Checked directly against each CLI's own help:

| agent | auto-approve flag | deny flag |
| --- | --- | --- |
| claude | `--allowedTools` | `--disallowedTools` |
| copilot | `--allow-tool` | `--deny-tool`, `--excluded-tools` |
| kiro | `--trust-tools` | *none found* |

Kiro exposes only `--trust-tools` (trust *these* tools), whose semantics
match the `--allowedTools` that ADR-0008 proved does not restrain. So for
at least one agent, a restricted role may not be expressible at all. An
abstraction that quietly runs a restricted role unrestricted would turn a
convenience into a security regression — the recipe's spec-authoring role
exists precisely to stop an agent implementing the feature instead of
writing the spec.

## Acceptance criteria

### AC-1 The agent axis includes kiro

WHEN an agent value of `kiro` is parsed THE SYSTEM SHALL accept it as a
valid agent alongside `claude` and `copilot`.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml agent_parses_kiro` exit 0

### AC-2 A turn's argv is built for the project's agent

WHEN a turn is built for a given agent THE SYSTEM SHALL emit that agent's
own invocation (claude `--print`, copilot `-p`, kiro `chat
--no-interactive`) rather than Claude's for all three.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml turn_argv_is_per_agent` exit 0

### AC-3 A restricted role is refused on an agent that cannot deny tools

IF a role grants a restricted tool set AND the selected agent has no
tool-denial mechanism THEN THE SYSTEM SHALL refuse the turn rather than
run it with unrestricted tools.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml restricted_role_refused_without_deny_support` exit 0

### AC-4 Tool denial is expressed per agent where it exists

WHEN a turn is built for an agent whose profile declares a deny flag THE
SYSTEM SHALL pass that flag with the role's denied tools (claude
`--disallowedTools`, copilot `--deny-tool`).

oracle: cmd `cargo test --manifest-path cli/Cargo.toml deny_flag_is_per_agent` exit 0

### AC-5 Turn output is parsed per agent

WHEN a turn completes THE SYSTEM SHALL read success and response text
using that agent's output shape, treating a non-JSON-emitting agent's
plain output as the response rather than failing to parse it.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml turn_output_parsed_per_agent` exit 0

### AC-6 Session continuity uses the agent's own resume form

WHEN a turn resumes a prior session THE SYSTEM SHALL pass the resume form
declared for that agent, and SHALL omit resumption for an agent whose
profile declares no resume form.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml resume_form_is_per_agent` exit 0

### AC-7 A missing credential names the right fix for that agent

WHEN a turn fails for want of a credential THE SYSTEM SHALL name the fix
for the selected agent (Claude's token, Copilot's `copilot /login`,
Kiro's `kiro-cli login`) rather than Claude's for all three.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml failure_hint_is_per_agent` exit 0

### AC-8 The chained turn record names the agent

WHEN a turn is recorded in the audit chain THE SYSTEM SHALL include which
agent produced it, so the trail distinguishes turns by agent.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml turn_record_names_the_agent` exit 0

### AC-9 Existing CLI tests stay green

THE SYSTEM SHALL leave the existing CLI test suite passing.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml` exit 0

## Out of scope

- Adding codex (keel has a driver; moor's turn path can follow the same
  shape later once the three here are proven).
- Changing `keel run`'s driver selection, SPEC-0010's image/agent
  resolution, or the `moor/copilot-<lang>` image build.
- Studio's own UX (reject, gate output in context, conversation→spec):
  this change unblocks them by making studio's agent half agent-agnostic,
  but does not implement them.
- Proving, by live experiment, that each agent's deny flag genuinely
  restrains it the way ADR-0008 did for Claude. This spec requires moor to
  *pass* the deny flag where it exists and to *refuse* where it does not;
  an ADR-0008-style live verification per agent is its own follow-up and
  is named as a known gap rather than assumed.
