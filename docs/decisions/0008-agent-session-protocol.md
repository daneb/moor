# ADR-0008: the agent reaches keel through an MCP server, not a scoped shell

**Status:** Accepted
**Date:** 2026-09-20

## Context

Before this change there were exactly two ways to put an instruction in
front of the sandboxed agent, and neither let you think a change through
first.

`moor shell` opens an interactive TTY and appends a single audit entry
whose entire redacted argv is the literal string `"shell"`
(`cli/src/commands/shell.rs`). Everything said in that session, and
everything the agent did in reply, is outside the hash chain. `moor
recipe` is the opposite: fully automated from a recipe file written
blind, with no conversational step at all.

Adding a conversational turn raises one hard question — **how does the
agent reach keel?** An agent that can build but can't run a gate has to
be babysat; an agent that can run `keel approve` can advance its own work
past the human checkpoints that are the entire point of keel.

The first draft of this answered it with a scoped shell:
`--allowedTools "Bash(keel gate *)"`, plus a moor-maintained allow/deny
list of keel verbs. That is exactly the control ADR-0003 already
rejected — a hand-written list that "would need moor to track Claude
Code's tool surface indefinitely and would rot every time that surface
changes." The rot was immediate: checked against the image's own Claude
Code 2.1.270, the real syntax is `Bash(git *)`, with a space, not the
colon form that draft assumed.

## Decision

moor ships its own MCP server into the sandbox (`mcp/`, installed as
`/usr/local/bin/moor-keel-mcp`, wired up by
`images/base/mcp-config.json`), exposing keel's *verification* verbs as
tools: `keel_gate` and `keel_next`.

`keel approve` is not a tool it defines. There is no allowlist entry to
maintain and no denial to enforce — the verb does not exist in the
agent's world. That is structural rather than configurable, the
distinction the README's Security section draws. MCP itself is a protocol
Anthropic maintains, rather than one moor has to track.

Two consequences worth naming:

- No role may reach a shell or a subagent (`cli/src/session.rs`). A shell
  hands `keel approve` straight back and undoes the whole argument. See
  the correction below for why *granting* no `Bash` turned out not to be
  enough.
- Each tool's argv is a compiled-in verb plus, at most, a spec slug that
  passes the same rule `Manifest::validate_name` applies, and a gate id
  matched against a fixed table so the caller's own bytes never reach the
  child process. An unexpected argument key is an error, not something
  ignored.

The server is hand-rolled JSON-RPC 2.0 over newline-delimited stdio with
`serde_json` only — no SDK, no listening socket, nothing crossing a
network. That is the same subprocess-and-pipes pattern `docs/ARCHITECTURE.md`
already describes for `docker`, `docker compose` and `gh`.

Around it, `moor ask` makes one turn: `claude --print --output-format
json`, resumed from a session id the *host* stores under
`~/.moor/projects/<name>/session` and only after it validates as a UUID,
so nothing in the agent's own response text can redirect the next turn.
Every turn appends one `agent-turn` entry to the project's existing hash
chain — role, granted tool set, session id, and SHA-256 hashes of the
prompt and response, no text — **before** the operator sees the answer.
Full text goes to a separate `transcript.jsonl`, redacted through
`audit::redact` the way `log_exec` already masks argv.

Three behaviours were verified against a live sandbox at Claude Code
2.1.270 rather than assumed:

- `--output-format json` returns `session_id`, `result`, `is_error`,
  `num_turns` and `permission_denials`.
- A failed turn returned `"is_error": true` alongside `"subtype":
  "success"`. **`subtype` is not a success signal**, so `turn_failed`
  keys on `is_error`.
- `--allowedTools` is variadic and swallows a trailing positional, so the
  prompt must precede it — the same trap `recipe::author_spec` documents.

## Correction (2026-09-21): granting is not withholding

The claim above — that `keel approve` is absent from the agent's world —
was **false as first shipped**, and found in live use rather than by
testing.

`--allowedTools` is an auto-approval list, not a restriction. Measured
against a running sandbox, Claude Code granted only `Read Glob Grep` ran
`Bash` anyway, returned its output, and recorded `permission_denials: []`.
Every permission mode was tried — `auto`, `manual`, `dontAsk`, `plan` —
and every one let it through; under `--print` there is nobody to answer a
permission prompt. With that shell, the `build` role reached
`keel approve --help` and quoted its first line back.

Only `--disallowedTools` denies. So every role now also passes
`--disallowedTools Bash BashOutput KillShell KillBash Task SlashCommand`
(plus the mutating and network tools for `brainstorm`), and with it the
agent reports having no shell tool at all. `commands::recipe`'s agent
steps carry the same list via `session::deny_shell_argv`, since they made
the identical assumption in a comment.

What this costs honestly: the control is now **half** structural. The MCP
server still means there is no `approve` tool to call, which is worth
having on its own — but a *list* is what keeps the verb from being reached
another way, and a list is exactly what ADR-0003 wanted to avoid. That is
a property of Claude Code's permission model rather than a decision moor
gets to make. moor over-denies deliberately: naming a tool that does not
exist costs nothing, and missing one that does costs the checkpoint.
`no_role_can_reach_a_shell_or_a_subagent` asserts the deny list reaches
the argv for every role, so this cannot silently regress.

## Consequences

The base image's build context is now the repo root rather than
`images/base`, because the image compiles the `mcp/` crate from source
(`images/build.sh`, `/.dockerignore`). Building it from source rather than
fetching a published binary is deliberate: the tool surface inside the
image can then never lag the protocol the CLI on the host expects.

`cli/src/commands/recipe.rs`'s `parse_recipe` became `pub` so
`session::emit_recipe` can put an agent-drafted recipe through the same
parser `moor recipe` uses before the host writes the file. One parser
means an emitted recipe can't be one `moor recipe` then refuses.

Still open, deliberately: nothing serialises a turn against a running
`moor recipe` (both can write one workspace), turns are request/response
with no streaming, and `images/base/Dockerfile` still installs
`@anthropic-ai/claude-code` unpinned — so the JSON shape `turn_failed`
keys on can move without warning. Each is its own spec.
