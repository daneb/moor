# moor

Run an AI coding agent on your Mac without giving it your Mac.

moor puts each project in its own locked-down Docker container and runs the
agent there (Claude Code today; Copilot and Kiro are supported too). The agent
can't see your files, can only reach the internet through an allowlist, and
can't approve its own work. moor stays outside, keeps a tamper-evident record
of what happened, and tells you what to do next.

<p align="center"><img src="docs/img/moor-sandbox.svg" alt="The agent runs in a sealed sandbox with no access to the host. The only way out is an egress proxy with an allowlist. moor, outside, holds the audit trail." width="800"></p>

## Why you might want it

Agents run commands, install packages and clone repos you haven't read. If
one is tricked or just wrong, you'd rather it break a throwaway container than
your laptop. And when it's done, you'd like to know what it did.

- **Contain it.** A bad run can damage one disposable container, not your Mac
  or your other projects.
- **Limit what leaks.** Traffic leaves only through a proxy that allows what
  you list (GitHub, your package registries) and logs everything, blocked
  requests included.
- **See what happened.** Commands, connections, pushes and approvals go into a
  hash-chained log on your Mac. `moor audit --verify` tells you if anyone
  edited it.
- **Stay in charge.** Work moves through checkpoints the agent can run checks
  for, but only you can approve.

Good for: trying an agent on code or a repo you don't fully trust, letting it
work unattended while you do something else, and being able to show a
teammate exactly what a change involved.

## How it works

1. `moor new` makes a sandbox container for the project, plus a second
   container that is its only route to the internet.
2. The agent works inside the sandbox. Your code lives in a Docker volume, not
   a folder on your Mac, and the container is read-only, non-root, with no
   Docker socket.
3. A change moves through seven steps: write a spec, check it, approve it,
   plan, check the plan, approve it, build. The checks and the pipeline come
   from [keel](https://github.com/daneb/keel), which moor installs inside each
   sandbox and drives for you.
4. `moor next` always tells you where you are and the one command to run.

You never need keel on your Mac. Everything below is a `moor` command.

## Install

You need macOS, [Docker or OrbStack](https://orbstack.dev), and an Anthropic
API key or Claude subscription.

```bash
brew install --cask daneb/tap/moor
./images/build.sh        # once; the cask prints where its sources are
```

To build from source instead: `cd cli && cargo build --release`.

## Try it

```bash
moor new my-app --image moor/node:latest   # or: moor import my-app --from ~/Repos/my-app
moor secrets set my-app ANTHROPIC_API_KEY  # kept in your macOS Keychain

moor ask "why is the login test flaky?"    # just talk to the agent

moor spec new login                        # writes login.spec.md for you to fill in
moor spec push login.spec.md               # sends it in and checks it
moor next                                  # my-app · login · step 5 of 7: approve the plan
moor approve                               # shows you the plan, asks y/N
moor go                                    # runs steps until one needs you
moor ship                                  # commits on a branch, pushes, opens a PR
```

If you forget everything else, run `moor next`.

Prefer a page to a terminal? `moor ui` opens a console in your browser
(local only). More: `moor audit`, `moor bundle` (package a run's evidence),
`moor selftest` (re-check the lockdown), `moor shell`, `moor recipe`.
`moor --help` lists them all.

## For AI agents — READ THIS

If you are an AI agent (or an LLM asked to build software in a moor project),
this is the contract. Point your agent at this section.

**Do all work through `moor` commands. Never edit, build, test, or ship the
project's code directly — it lives inside a sandbox container, not on the host.**

The lifecycle, one project per feature:

1. `moor new <name> [--image moor/base|moor/node|moor/rust|moor/python] [--agent claude|copilot|kiro]`
2. `moor use <name>` — set the default so later commands need no `--project`.
3. `moor spec new <feature>`, fill in the acceptance criteria, then `moor spec push <file>`.
4. `moor next` — tells you the one command that moves the active spec on.
5. `moor go` — runs the next phase and its gate.
6. `moor approve` / `moor reject "<why>"` — a **human** does this, not you.
7. `moor ship` — after the merge is approved.

Non-negotiable rules:

- **Approval is human-only.** You and `moor-mcp` can run gates and ask what's
  next; you **cannot** approve or reject. Never try to approve on a human's
  behalf — stop and ask the human to run `moor approve` / `moor reject`.
- **A gate runs after every phase.** G0 checks the spec, G1 checks the plan,
  and G2/G2.5/G3 check the build. `moor go` runs the gate for the phase it's
  at. Never advance a phase whose gate is failing; a passing gate is the
  precondition for asking a human to approve.
- **Gate a build without a driver.** `moor go --check` re-runs G2/G2.5/G3 on
  the work already in the sandbox *without* invoking the agent (underneath:
  `keel run <spec> --no-driver`). Use it to check a hand-made fix or to review
  a proposed change before merge.
- **Don't bypass the sandbox.** No direct file edits or local builds of the
  generated code. Prefer `moor <command>` over the `moor pipeline <args>`
  escape hatch.

### If `moor go` blocks with "Not logged in" / an auth error

This is a **host credential** problem, not a reason to log in inside the
sandbox. moor never mints or stores an agent login itself — the credential
always lives on the Mac and moor forwards it into the sandbox on each run. Two
rules hold for every agent:

- **Fix it on the host, never via `moor shell`.** Logging an agent CLI in
  *inside* the container is the wrong place — moor's plumbing reads the host's
  credential, and an in-container login does not survive a rebuild (`moor up`,
  an image change) anyway. The code lives in a volume; the agent's credentials
  do not.
- **Resolution precedence** (`secrets::resolve_into_env`): an already-set env
  var in your shell wins first, then a `moor secrets`-managed Keychain entry,
  then the agent's own auto-forwarded host credential below. A stale env var
  therefore silently beats a fresh host login — unset it if in doubt.

First check the project's **agent matches the credential you set**: `moor.yaml`'s
`agent:` (and the default driver in `.keel/keel.toml`) must be the agent you
authenticated. A kiro credential does nothing for a `claude` project, etc.
Then, by agent:

**Claude** (`agent: claude`) — needs `CLAUDE_CODE_OAUTH_TOKEN` *or*
`ANTHROPIC_API_KEY`.
- OAuth token (a claude.ai subscription): run `claude setup-token` on the host.
  When both are set, the `claude` CLI prefers `CLAUDE_CODE_OAUTH_TOKEN` — that
  precedence is the CLI's own, not moor's.
- API key: `moor secrets set <project> ANTHROPIC_API_KEY` (stored in the
  Keychain), or export it in your shell.

**Copilot** (`agent: copilot`) — needs `COPILOT_GITHUB_TOKEN`.
- Run `copilot /login` on the host. The Copilot CLI stores its own OAuth
  (`gho_`) token in the macOS Keychain under service `copilot-cli`, and moor
  auto-forwards it as `COPILOT_GITHUB_TOKEN` when nothing more explicit resolved
  it. The CLI reads `COPILOT_GITHUB_TOKEN` then `GH_TOKEN`.

**Kiro** (`agent: kiro`) — needs `AWS_BEARER_TOKEN_BEDROCK`.
- Run `kiro-cli login` on the host (add
  `--license pro --identity-provider <start-url> --region <region>` for Identity
  Center). moor reads the host's freshest unexpired IAM Identity Center SSO
  token from `~/.aws/sso/cache/*.json` and injects it as
  `AWS_BEARER_TOKEN_BEDROCK` on each run. An expired host login re-blocks it;
  re-run `kiro-cli login`.
- Caveat: this path reads IAM Identity Center SSO cache files. A Builder ID /
  GitHub / Google ("free") login may store its credential elsewhere; if
  `moor secrets status` and a confirmed host login still leave `moor go`
  blocked, that credential may not land in `~/.aws/sso/cache`.

`moor secrets status <project>` shows where each declared credential currently
resolves from (shell env, Keychain, or missing) — run it first when unsure.

## What it isn't

- **macOS only** for now, built around Docker/OrbStack.
- **Not a VM.** Containers share your Docker host's kernel, so a kernel-level
  escape isn't defended against ([why](docs/decisions/0001-container-runtime-choice.md)).
- **Not a way to make the agent correct.** It limits the damage and shows
  you what happened.
- **Not independently audited.** The security claims are backed by tests and
  `moor selftest`, but that's self-assessed. Known gaps:
  [docs/SECURITY-NOTES.md](docs/SECURITY-NOTES.md).

## Learn more

- [Walkthrough](docs/WALKTHROUGH.md): a real idea-to-PR run, bugs included
- [Architecture](docs/ARCHITECTURE.md) · [threat model](docs/THREAT-MODEL.md)
- [Adopting an existing project](docs/MIGRATING.md) · [recipes](docs/RECIPES.md)
- [Images](docs/IMAGES.md) · [agent transport](docs/MCP.md) · [CI](docs/CI.md)
- [Design decisions](docs/decisions/) · [security notes](docs/SECURITY-NOTES.md)
- [Contributing](CONTRIBUTING.md) · [history](docs/HISTORY.md) · [changelog](CHANGELOG.md)

MIT licensed.
