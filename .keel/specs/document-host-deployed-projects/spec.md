---
id: SPEC-0002
slug: document-host-deployed-projects
schema: keel.spec/1
status: draft
scope:
  - "docs/MIGRATING.md"
  - "docs/ARCHITECTURE.md"
budget:
  criteria: 8
  lines: 120
verified_at: 2026-09-17
---

# Document the pattern for projects with a host-deployed service

## Context

Every documented moor lifecycle (ARCHITECTURE.md's diagram, WALKTHROUGH.md's
ideation-to-shipped walkthrough, MIGRATING.md's import flow) ends at `git
push` — code leaves the sandbox through the egress proxy's `github.com`
allow-list, and that's the whole story. That fits a library/CLI project
(build, test, lint, push) exactly. It does not fit a project whose actual
"shipped" artifact is a **running host-side service** — one imported
project (`personil`) is a `docker-compose.yml`-defined stack
(`personil-server`, plus standalone `ntfy`/`freshrss` containers) bound to
loopback and a Tailscale interface, with `restart: unless-stopped` — none
of which the sandbox can touch (`moor selftest` explicitly checks for *no*
`docker.sock` in the sandbox, by design — THREAT-MODEL.md). Nothing in the
docs says what to do once code merged and pushed from the sandbox needs to
actually reach that host-side service. This was discovered by trial and
error: work happened directly on the host repo instead, because there was
no documented alternative.

## Acceptance criteria

### AC-1 MIGRATING.md states the deploy step for a project with its own host-side service

WHEN a reader of `docs/MIGRATING.md` reaches the end of the import flow for
a project that also runs a host-deployed service (e.g. its own
`docker-compose.yml` outside the sandbox) THE SYSTEM SHALL document that
the sandbox's role stops at `git push`, and that reconciling the host
checkout (`git pull` + redeploy, e.g. `docker compose up -d --build`) is a
separate, host-side step the sandbox cannot perform.

oracle: cmd `grep -q 'host-deployed service' docs/MIGRATING.md` exit 0

### AC-2 ARCHITECTURE.md's diagram or components section names the boundary explicitly

WHEN a reader of `docs/ARCHITECTURE.md` looks for why the sandbox cannot
also restart a project's own docker-compose stack THE SYSTEM SHALL state
that host-side deployment is out of scope for the sandbox by the same
no-`docker.sock` design decision that rules out bind mounts, not an
oversight to be worked around.

oracle: cmd `grep -q 'host-deployed service' docs/ARCHITECTURE.md` exit 0

## Out of scope

_This spec is documentation only. It does not add a `moor deploy`/`moor
promote` verb, or any host-side Docker capability to the sandbox — see
THREAT-MODEL.md's no-`docker.sock` rationale, which this spec treats as
correct and unchanged._
