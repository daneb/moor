# Abandoned specs

Specs in this folder were gated (G0 passed) but never approved or built, and
are not being pursued. They are moved here — rather than deleted — so the work
and its G0 evidence stay in git history and can be revived by moving the folder
back up to `.keel/specs/<slug>/`.

`keel spec list` / `keel next` only enumerate direct child folders of
`.keel/specs/` whose `spec.md` is a file, so nothing in here surfaces in the
active pipeline.

This is a manual stopgap. The durable fix is a first-class `abandoned` terminal
state in keel — see the `spec-abandonment` spec.

## Retired on 2026-11-02

| slug | id | why |
|------|----|-----|
| compose-project-namespace | SPEC-0001 | Superseded; compose namespacing handled elsewhere. Never approved. |
| document-host-deployed-projects | SPEC-0002 | Docs-only backlog item, not being pursued. Never approved. |
| agent-session-protocol | SPEC-0003 | Folded into agent-session-abstraction (SPEC-0012). Never approved. |
| studio-multi-project-console | SPEC-0004 | Superseded by studio-authoring / studio-decisions. Never approved. |

Edit the "why" lines above if any of these is inaccurate.
