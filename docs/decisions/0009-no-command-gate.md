# ADR-0009: no rules-based command gate in `moor run`

**Status:** Accepted
**Date:** 2026-09-29

## Context

SPEC-0008 and SPEC-0009, and a follow-up, built a rules-based gate that
ran before `moor run` handed a command to `docker exec`. The plan was
for a model to sit behind it later. The work was merged in PRs #6, #7
and #8, and step 4 (#9) was in review, when we asked what the gate
actually protects.

- It denied commands mentioning credential paths (`.ssh`, `.aws`,
  `.pem`), `sudo`, or `docker.sock`, and recursive force-removes of `/`,
  `/workspace` or `~`.
- A flagged command could be run anyway by answering y/N at a real
  terminal.
- A per-session "task context" added `out_of_scope` and `path_escape`
  rules.

## Why it was reverted

**The agent never went through it.** The gate only saw argv passed to
`moor run` and `moor keel`, which the operator types on the host. Agent
turns (`moor ask`, `moor recipe`) start the agent with their own
`docker exec`, and its tool calls then run inside the container without
passing the gate. So it filtered the operator's commands, not the
agent's.

**What it looked for isn't in the container.** THREAT-MODEL.md already
rules out a filesystem breakout by design: no bind mounts, no
`docker.sock`, no shared volumes, and a read-only root filesystem. The
host's `.ssh` and `.aws` are not in the sandbox. The secrets that are
there live in environment variables, and `printenv` passed every rule.
`sudo` only gets root inside a throwaway container. `rm -rf /workspace`
wipes a named volume in a container that is disposable by design. The
task-context rules only policed paths inside that same container.

What was left was a guard against the operator mistyping their own
`rm -rf`. That doesn't justify a rules engine, a y/N prompt, a
session-context format, and a planned model.

## Decision

We removed the gate completely: `cli/src/decision.rs`, the gate and y/N
prompt in `run_cmd.rs`, `audit::log_decision`, and `docs/adr/0001` to
`0003`. `cli/` and `docs/` are back to their state at `0dc44c5`. We kept
the `.keel/` spec, run and chain records for SPEC-0008 and SPEC-0009,
because the chain is append-only history.

Security work should go where the container can actually affect the
host:

- the structural guarantees in THREAT-MODEL.md, and keeping them proven
  (`moor selftest`, posture attestation);
- egress: the proxy allow-list and the canary;
- code leaving the sandbox (`git push` tagging);
- **data from the container that host code reads or acts on**: the
  keel sink and egress-log folding, the recipes `moor ask` emits, and
  the `$EDITOR` round-trip. This is where a real escape would happen.

## Test for future proposals

Before adding a control, name the host-side asset it protects and the
path by which the container reaches it. If the answer is "something
inside the container," the container boundary already answers it.
