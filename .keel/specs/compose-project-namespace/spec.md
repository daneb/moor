---
id: SPEC-0001
slug: compose-project-namespace
schema: keel.spec/1
status: draft
scope:
  - "cli/templates/project.compose.yml.tmpl"
  - "cli/src/compose.rs"
budget:
  criteria: 8
  lines: 120
verified_at: 2026-09-17
---

# Namespace the generated compose project name to prevent collisions

## Context

`cli/templates/project.compose.yml.tmpl` never sets a top-level `name:` key.
Docker Compose falls back to the directory basename of the compose file's
parent (`~/.moor/projects/<name>/`), which is always exactly the moor
project's own `{{PROJECT_NAME}}`. If a project of the same name is imported
from a host repo whose own `docker-compose.yml` *also* has no explicit
`name:` (so it too falls back to its directory's basename), and that
directory happens to share the same basename as the moor project — the
ordinary case for `moor import <name> --from ~/Repos/<name>` — both compose
files resolve to the identical Compose project name. Docker then reports
them as one merged project (`docker compose ls` shows a single project
spanning both config files), and any `docker compose` command scoped to
that name — including whatever `moor up`/`down` runs internally for
sandbox lifecycle — can no longer distinguish the target repo's own stack
from the sandbox/egress pair. This was observed for real against the
`personil` project: `moor import personil --from ~/Repos/personil` merged
with `~/Repos/personil/docker-compose.yml`'s own (unnamed) project.

## Acceptance criteria

### AC-1 Generated compose project name is namespaced against the target repo's own project name

WHEN `compose::render` renders a project's compose file THE SYSTEM SHALL
set a top-level Compose `name:` of `moor-{{PROJECT_NAME}}`, distinct from
`{{PROJECT_NAME}}` alone, so it cannot collide with a same-named project's
own directory-basename-derived Compose project name.

oracle: cmd `grep -q '^name: moor-{{PROJECT_NAME}}$' cli/templates/project.compose.yml.tmpl` exit 0

## Out of scope

_Detecting or warning about an existing `docker-compose.yml` in the
imported repo at `moor import` time is a separate concern — see spec
`document-host-deployed-projects` for the documentation side of that. This
spec only closes the name-collision hole itself._
