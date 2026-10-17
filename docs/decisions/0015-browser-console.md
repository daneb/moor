# ADR-0015: `moor ui` — the console in a browser, on loopback, for one run

**Status:** Accepted
**Date:** 2026-10-17

## Context

`moor studio` (ADR-0008's follow-on) put every project in one terminal:
status, stages, a chained turn with the agent, and approval behind two
distinct keys. It deliberately opens no socket, binds no port and runs no
daemon. That is the right posture for a terminal console, and it is also
why a terminal is the only place the whole picture exists.

The operator wanted the same picture — and the same actions — as a page:
every project at a glance, where each spec stands as a map they can step
into, the artefacts and gate evidence next to the decision they inform,
the audit trail readable rather than `jsonl`, and agent, driver and
credential setup in one place instead of across `moor.yaml`, the
Keychain and `keel.toml`.

A page needs an HTTP server. The question was how to add one without
giving up what the studio's "no socket" rule protects.

## Decision

`moor ui [--port N] [--no-open]` (`cli/src/ui/`) serves an embedded
single-page console. It holds the line in five places:

1. **Loopback only, for exactly one run.** It binds `127.0.0.1`, and
   only while the command runs. No launchd entry, no background process,
   nothing listening after Ctrl-C. The `Host` header must name loopback,
   so a page elsewhere that resolves its own name to `127.0.0.1` (DNS
   rebinding) is refused.
2. **A per-launch key, carried where a server never sees it.** 192 bits
   from `/dev/urandom`, printed once in the URL's *fragment*. Fragments
   do not travel in requests or logs; the page reads it and sends it back
   as a header on every `/api` call. Another origin on the same machine
   cannot read the fragment and cannot set the header cross-origin, so it
   cannot drive the console. The one route that streams (`/stream`, a
   browser `EventSource`, which cannot set headers) accepts the key as a
   query parameter instead; `Referrer-Policy: no-referrer` and a
   `connect-src 'self'` policy keep that URL on the machine.
3. **Every action is the `moor` binary, run as a child.** `moor go` from
   the page is byte-for-byte `moor go` from a terminal: same checks, same
   audit entries, same refusals. The page never assembles a command line.
   It posts a *kind* from a fixed table in `ui/api.rs` plus validated
   fields (project and spec names through `manifest::validate_name`,
   paths that must exist, roles that must parse); the table turns those
   into argv. A kind not in the table cannot be run. Prompts for
   confirmation are answered only where the operator confirmed on the
   page (`--yes`); a secret's value travels on the child's stdin, never
   in argv, so it is never in `ps`, the job list or the chain.
4. **Reads go through the audited path.** Everything read out of a
   sandbox is `docker exec`, logged through `audit::log_exec` under
   `kind: "ui"`, as the studio logs its own reads under `studio`. Host
   files (manifests, the chain, transcripts) are read in place. One
   consequence needed a rule: `guide::pick_active` treats "the spec the
   chain touched last" as the active one, and a read mentions the slug.
   Looking at a spec in a browser is not working on it, so `pick_active`
   now skips `kind: "ui"` entries — a console read never redirects `moor
   go` at whatever was looked at last. Asserted in
   `a_console_read_does_not_make_a_spec_the_most_recent`.
5. **Sandbox text is bytes, not markup.** Everything from a sandbox passes
   `studio::render::sanitize` before it leaves the server, and the page
   renders it as text; its markdown renderer escapes before it
   transforms. The page loads no remote script, style or font: the
   content-security policy is `default-src 'self'`, and the whole page is
   three files embedded in the binary (`cli/ui/`).

Approval keeps the studio's shape: two distinct clicks, the second on a
confirmation that names what is being approved; a rejection needs a
reason before it can be recorded. The journey map is `guide`'s own
seven-step table with a ship node after it, so the page and `moor next`
cannot disagree about where a spec stands.

## Consequences

- The HTTP server is hand-rolled over `std::net` (`ui/http.rs`), like
  the MCP server is over stdio: no new dependency, one request per
  connection, `Content-Length` bodies, server-sent events for a job's
  output. It is not a general web server and does not try to be.
- `moor ui` is a second front end over the same commands, not a second
  implementation of them. A command that gains a flag needs the job table
  to learn it; a command that changes its output needs nothing, since
  the page shows output as text.
- Reads cost chain entries. Loading a project is four `docker exec`s
  (`keel next`, `keel runs`, two `git status`), a spec three more. The
  page loads on navigation and after a job, not on a timer, so the trail
  grows with what the operator looked at, not with how long a tab was
  open.
- `MOOR_UI_DEV=<dir>` makes the server read the page from that directory
  on each request instead of the embedded copy, for working on the page
  without rebuilding. It is for development only and is never set by
  anything moor does.
