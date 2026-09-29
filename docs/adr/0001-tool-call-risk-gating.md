# ADR-0001: Tool-call risk gating is enforced host-side, before `docker exec`

## Status

Accepted. Implemented by keel spec `tool-call-risk-gate` (rules only; see
"Deferred" below for the model, daemon, and "ask" workflow a later spec
would add).

## Context

A spike proposed flagging risky agent tool calls with a local decision
model (a small encoder with Jev-style multi-question heads), gated by a
"PreToolUse"-style hook: the agent's own harness calls a decision daemon
before running a tool, and the daemon's verdict decides whether the call
proceeds.

`docs/THREAT-MODEL.md` already answered a narrower version of this
question. Its "Why not a git pre-push hook" section rejects a hook baked
into the sandbox image for the same reason: *"a git hook runs inside the
sandbox, under the same account that controls its own git config — the
agent... can simply reconfigure or bypass it, so it would be a control
that looks like a boundary but isn't one."*

The agent this spike targets (Claude Code) runs **inside** the sandbox
container, same as the git client that reasoning was written about. A
`PreToolUse` hook is agent-side configuration, evaluated by the same
process tree the sandbox is meant to constrain. It fails for the identical
reason a pre-push hook does: nothing stops a compromised or prompt-injected
agent from editing its own hook config, pointing it at `/bin/true`, or
simply not going through whatever invokes it.

Moor's actual host/sandbox boundary for a `moor run <project> -- <cmd>`
invocation is `cli/src/commands/run_cmd.rs::run`. It is host-side code: it
resolves the project's manifest, then execs `docker exec <container> <cmd>`
from the `moor` CLI process itself, and audits the result afterward. That
is the one place a command from an agent-issued `moor run` genuinely
crosses from untrusted (inside the container) to trusted (the host), and,
before this change, it does not gate anything there — it execs
unconditionally.

The MCP server (`mcp/`) and Claude Code's own `--disallowedTools` are the
other two controls that touch the agent's action surface today. Both are
structural allow/deny lists fixed at container-start time, not something
consulted per call, and both already exist independently of this ADR.

## Decision

Risk verdicts are decided host-side, in `moor run`, before it constructs
the `docker exec` argv. `cli/src/decision.rs` holds a fixed, deterministic
rule table (`decision::evaluate`) that returns `Allow` or `Deny`; a `Deny`
is logged to the project's audit chain and returned as the error `run`
propagates, so the denied command is never handed to `docker exec`.

Any in-sandbox signal — a Claude Code hook, keel's own sink, an MCP
tool-call log — is admissible only as a *signal* to whatever eventually
sits behind `decision::evaluate` (see keel's sink in
`docs/THREAT-MODEL.md`, "keel's payloads come out, as claims"). It is
never the enforcement point, and a verdict it produces is never trusted
on its own to allow or deny anything.

### The question schema this rule table stands in for

The spike's five `noul` questions (`destructive`, `exfiltration`,
`credential_access`, `boundary_escape`, `out_of_scope`) and one `score`
(`blast_radius`) describe what a future model backend would answer.
`decision::evaluate` answers three of the five deterministically today —
`credential_access`, `boundary_escape`, `destructive` — as fixed pattern
matches against the exec argv, each denying independently and each
carrying its own `rule_id` into the audit entry. `exfiltration` is already
covered by a different, existing control (the egress proxy's allow-list
and canary-token watch, both outside the sandbox) and is not this rule
table's job. `out_of_scope` and `blast_radius` need the approved-task
summary this ADR's open questions (below) leave unresolved, and are left
for the model.

## Consequences

- A rule that matches denies unconditionally; there is no `Ask` verdict
  yet, and no UI to act on one. Design principle 3 from the spike
  ("escalate on uncertainty") is deferred along with the model that would
  produce a probability to escalate on.
- The gate only sees commands that pass through `moor run`. It does not
  see commands typed inside an interactive `moor shell` session, or verbs
  dispatched to the in-sandbox MCP server — both pre-existing blind spots,
  not new ones introduced here.
- False positives are refusals with no override path yet other than not
  running that exact command through `moor run` — acceptable while the
  rule table is three narrow, high-confidence patterns; would need
  revisiting before the table grows.

## Deferred

- The local decision model, its ONNX export, and the `ort`-backed daemon.
- An `Ask` verdict and any human-in-the-loop workflow for it.
- Dataset collection, teacher labelling, training, and calibration.
- Any agent other than Claude Code (e.g. Kiro), and any surface other than
  `moor run`.
- Open questions 2–4 from the spike (agent coverage beyond Claude Code,
  the approved-task summary's source and format, and log retention/
  redaction rules for training data) — unresolved, and blocking before the
  model or dataset work starts, not before this ADR's rules-only gate.
