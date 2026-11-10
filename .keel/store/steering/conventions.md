---
id: CONV-0001
scope: repo
owner: human
verified_at: 2026-09-17
---

# Conventions

House rules that apply to every change in this repository. Keep this list short
and specific: a rule nobody can violate mechanically is a rule that will be
violated. Where a rule can be checked by a command, say so — in Phase 3 those
become gate checks and stop costing context.

## Working agreement

- Match the surrounding code. Naming, comment density and idiom are local
  conventions, not global ones.
- Change the smallest surface that solves the problem. If a fix needs a wider
  blast radius, say so before making it, not after.
- A test that mocks away the behaviour under test is worse than no test.

## Naming: keel is internal

moor's users never type `keel`; everything is a `moor` command. keel
(github.com/daneb/keel) is the pipeline engine moor installs inside each
sandbox at a pinned version (`KEEL_VERSION` in `images/base/Dockerfile`).

- To drive a project's pipeline, use `moor next`, `go`, `approve`, `reject`,
  `ship`. `moor pipeline <args>` is the hidden escape hatch to the sandbox's
  keel. When a user says "next step" or "approve", they mean the `moor` command.
- `moor-mcp` (`mcp/`) is the agent's route to checks inside a sandbox
  (`check`, `next`). It cannot approve, by design.
- A bare `keel` on this machine's PATH works on THIS repo's `.keel/` (specs,
  store, generating `CLAUDE.md`/`AGENTS.md`): development tooling only. Run
  `keel store render` after editing `.keel/store/`. It is a different copy
  from the sandbox's.

## Steering agents working in a moor project

The contract for any AI agent (or LLM) asked to build software here. Do all
work through `moor` commands: the project's code lives inside a sandbox
container, not on the host, so never edit, build, test, or ship it directly.

Lifecycle, one project per feature:

1. `moor new <name> [--image moor/base|node|rust|python] [--agent claude|copilot]`
2. `moor use <name>` — set the default so later commands need no `--project`.
3. `moor spec new <feature>`, fill in the acceptance criteria, `moor spec push <file>`.
4. `moor next` — the one command that moves the active spec on.
5. `moor go` — runs the next phase and its gate.
6. `moor approve` / `moor reject "<why>"` — a human does this, not the agent.
7. `moor ship` — after the merge is approved.

Non-negotiable:

- **Approval is human-only.** The agent and `moor-mcp` can run gates and ask
  what's next; they cannot approve or reject. Never approve on a human's
  behalf — stop and ask the human to run `moor approve` / `moor reject`.
- **A gate runs after every phase.** G0 checks the spec, G1 the plan,
  G2/G2.5/G3 the build. Never advance a phase whose gate is failing; a passing
  gate is the precondition for asking a human to approve.
- **Gate a build without a driver** with `moor go --check` (underneath:
  `keel run <spec> --no-driver`) — to check a hand-made fix or review a change
  before merge.
- **How code leaves the sandbox:** only `moor ship`. It commits on a branch
  inside the sandbox, streams that branch out as a `git bundle` over `docker
  exec` stdout, then fetches and `git push`es it from the host with the host's
  own `gh` credential, and opens a PR — targeting a branch, never a direct push
  to the trunk. The sandbox never holds a GitHub token (ADR-0014).

### When `moor go` blocks with "Not logged in" / an auth error

A host-credential problem, not a reason to log in inside the sandbox. moor
forwards the host's credential into the sandbox on each run; an in-container
login is the wrong place and does not survive a rebuild.

- Check the project's `agent:` in `moor.yaml` (and the default driver in
  `.keel/keel.toml`) matches the credential you set — a copilot credential
  does nothing for a `claude` project.
- **Claude:** needs `CLAUDE_CODE_OAUTH_TOKEN` (run `claude setup-token` on the
  host) or `ANTHROPIC_API_KEY` (`moor secrets set <project> ANTHROPIC_API_KEY`).
- **Copilot:** run `copilot /login` on the host; moor auto-forwards its Keychain
  token as `COPILOT_GITHUB_TOKEN`.
- `moor secrets status <project>` shows where each credential resolves from.

## Rules

_Add rules as you find yourself repeating them. One line each, imperative mood._
