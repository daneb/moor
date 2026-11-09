# Build everything through moor

All software work in this workspace runs inside moor's sandboxed, per-project
pipeline. Never write, edit, build, test, or ship project code directly in this
repo. Instead, drive the work with `moor` CLI commands so every change happens
inside a locked-down container with an audit trail.

## Per feature/project, use this flow

1. `moor new <name> [--image moor/base|moor/node|moor/rust|moor/python] [--agent claude|copilot|kiro]`
   — scaffold a new sandboxed project. Each distinct piece of work gets its own project.
2. `moor use <name>` — make it the default so later commands need no `--project`.
3. `moor spec new <feature> [--title "..."] [--scope "src/**"]` — write a starter spec,
   then fill in the acceptance criteria. `moor spec push <file>` sends it into the sandbox.
4. `moor next` — show where the active spec stands and the single command that moves it on.
5. `moor go` — run the next step until something needs a human. Each phase is gated:
   G0 checks the spec, G1 checks the plan, and G2/G2.5/G3 check the build. `moor go`
   runs the gate for the phase it's at; never skip a gate.
6. `moor approve` / `moor reject "<why>"` — a HUMAN acts on what the spec is waiting on.
7. `moor ship` — once the merge is approved, commit/push the work.

Supporting commands:
- `moor ask -- "<question>"` — one agent turn (talk to the agent, no approval power).
- `moor go --check` — re-gate the build (G2/G2.5/G3) on work already in the sandbox,
  WITHOUT the agent/driver. Underneath this is `keel run <spec> --no-driver`: use it to
  check a change you (or a human) fixed by hand, or to review a proposed change before merge.
- `moor up <name>` / `moor down <name>` — start/stop a sandbox. `moor next --all` — every spec's step.

## Rules

- Approval is human-only. The agent and `moor-mcp` can run gates and ask what's next,
  but CANNOT approve or reject. Never attempt to approve on the human's behalf — stop and
  ask them to run `moor approve` / `moor reject`.
- Run the gate after each phase (G0 → G1 → G2/G2.5/G3). Treat a passing gate as the
  precondition for asking for approval; never advance a phase with a failing gate.
- Prefer `moor <command>` over the hidden `moor pipeline <args>` escape hatch.
- One project per distinct feature/app; keep varied work in separate sandboxes.
- When I say "next step", "approve", or "reject", I mean the `moor` command, not raw keel.
- Don't bypass the sandbox: no direct file edits or local builds of the generated code.
