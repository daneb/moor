# ADR-0003 — Task-Context Wire Format

**Status:** Accepted
**Date:** 2026-09-29
**Deciders:** Dane Balia

## Context

`decision::evaluate` currently takes only the raw command slice. It can answer credential, boundary, and destructive-wipe questions deterministically, but cannot answer:

- **`out_of_scope`** — is this command within the verbs the approved spec expected?
- **`blast_radius`** — how broad is the potential impact?

Both require knowing what the agent was approved to do. This ADR defines the wire format that stamps that context at session start.

## Decision

### TaskContext schema

Written to `~/.moor/sessions/<session_id>/context.json` at session start.

```json
{
  "session_id": "01932f4a-7b2c-7e1d-8f3a-2d4c6e8a0b1c",
  "spec_title": "Add login endpoint",
  "gate": "standard",
  "expected_verbs": ["read_file", "write_file", "bash", "cargo_check", "cargo_test"],
  "allowed_paths": ["/workspace"],
  "forbidden_paths": ["/workspace/.git", "/etc", "/root", "/home"],
  "started_at": "2026-09-29T10:00:00Z"
}
```

| Field | Type | Description |
|---|---|---|
| `session_id` | UUIDv4 | Ties audit log entries to a session |
| `spec_title` | string | Label from the Keel spec |
| `gate` | `"minimal"` \| `"standard"` \| `"strict"` | Gate level; set by the recipe, defaults to "standard" |
| `expected_verbs` | string[] | MCP tool-call names the agent is approved to use |
| `allowed_paths` | string[] | Path prefixes the agent may touch |
| `forbidden_paths` | string[] | Explicit denies overriding allowed_paths |
| `started_at` | ISO 8601 | Session start timestamp |

### Gate levels

| Gate | Behaviour |
|---|---|
| `minimal` | credential_access + boundary_escape + destructive_wipe only |
| `standard` | All deterministic rules + out_of_scope + path_escape |
| `strict` | All rules + blast_radius logged |

### Updated evaluate signature

`pub fn evaluate(cmd: &[String], ctx: Option<&TaskContext>) -> Verdict`

Option keeps existing callers backward-compatible (pass None).

### Loading the context in `moor run`

`moor run` looks up the project's current `moor ask` session id (`~/.moor/projects/<name>/session`) and loads `~/.moor/sessions/<session_id>/context.json`. Both paths are host-side, so the agent cannot choose or widen its own scope.

- No session id, or no context file for it: `ctx` is `None`, and only the context-free rules apply.
- A context file that is unreadable, malformed, or stamped with a different `session_id`: `moor run` fails before `docker exec`. Treating it as absent would silently switch off `out_of_scope` and `path_escape`.
- An `out_of_scope` or `path_escape` match goes through the same Ask resolution as any other rule (ADR-0002).

## Consequences

- out_of_scope and path_escape are now implementable as deterministic rules
- TaskContext becomes the input envelope for the future Jev model
- Audit log gains session_id for per-session replay
- blast_radius score (deferred) will provide Jev training labels

## Implementation order

1. TaskContext struct + serde ✓
2. evaluate signature update ✓
3. out_of_scope + path_escape rules ✓
4. run_cmd::gate loads context file from disk ✓
5. moor keel stamp sub-command
6. blast_radius logging (strict gate)
7. (Later) Jev model training data
