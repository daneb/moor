# `moor ui` — the console in a browser

Everything `moor next`, `go`, `approve`, `reject`, `ship`, `ask`, `view`,
`audit`, `bundle`, `secrets` and `new`/`import` do, as one page.

```bash
moor ui                 # opens http://127.0.0.1:7474/#<key> in your browser
moor ui --port 0        # pick a free port
moor ui --no-open       # print the URL only
```

The key after `#` is this run's only way in; open the exact URL printed.
The server lives as long as the command does. Ctrl-C stops it.

## Reading the page

- **Harbour** — every project, the ones waiting on you first. The dots
  under a project are the seven steps plus ship: green done, yellow now.
- **Journey** — the map for one spec. Click a node to open it: the
  artefact on the left, the checks, approvals and the action for that
  step on the right. The card above the map is `moor next` in plain
  words, with its one command as a button.
- **Specs** — every spec and its step. *New spec* gives you the starter
  file with its rules, then pushes and runs G0. *Write a recipe* runs
  the whole pipeline from a loose description, pausing at approvals.
- **Agent** — `moor ask`. Brainstorm is read-only; build can edit and
  run checks, never approve. *Turn into a recipe* writes an answer out
  as a recipe file if it parses.
- **Runs & evidence** — each run's gates, evidence files and trajectory.
  *Bundle evidence* is `moor bundle`.
- **Audit** — the chain, filterable by kind. *Verify chain* is
  `moor audit --verify`; *Self-test* re-checks the hardening.
- **Settings** — agent provider (Claude, Copilot, Kiro) with credential
  status and host login steps, keel's default driver, Keychain secrets,
  egress allowlist, resources, image. Manifest changes apply on the next
  *Restart sandbox*.
- **Activity** (bottom) — every action's output as it runs. ⌘K opens the
  command palette; `r` refreshes.

## Rules it keeps

- Approval is two clicks; a rejection needs a reason. Both are recorded
  under your git identity, like the command line.
- Every action *is* the `moor` binary, run as a child. Same checks, same
  refusals, same audit entries.
- Every sandbox read is in the audit chain as `kind: "ui"`. Reads do not
  make a spec "active": `moor go` follows what you worked on, not what
  you looked at.
- A secret's value travels on stdin, never in a command line.
- Loopback only; a page on another origin cannot drive it.

Why it is built this way: [ADR-0015](decisions/0015-browser-console.md).
