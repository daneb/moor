# ADR-0012: Copilot auth in the sandbox is the host's device-flow token, discovered from the Keychain

**Status:** Accepted (moor host side shipped; keel driver preflight is a sibling, in keel's repo)
**Date:** 2026-10-01

## Context

Running the GitHub Copilot CLI inside the sandbox needs a credential, and
the obvious idea — "store a token with `moor secrets`" — does not reliably
work, for reasons that are GitHub's, not moor's:

- **Classic PATs (`ghp_`) are rejected outright** by Copilot CLI.
- **Fine-grained PATs usually fail too** — Copilot requires an
  account-level permission only its own OAuth device flow grants
  (confirmed against GitHub's own issues/discussions, not assumed).
- The mechanism Copilot CLI actually honours is **`copilot /login`**, the
  OAuth device flow — the same thing other coding-agent CLIs (Kiro) use.

So the credential the operator can reliably obtain is a device-flow token,
and they get it by running `copilot /login` once on the host (the sandbox
has no interactive session to run the flow in, by design). The question is
how that host credential reaches the isolated sandbox seamlessly.

Investigated locally against a real logged-in host:

- `copilot /login` does **not** write a token to a file in `~/.copilot`
  (`config.json` records only `loggedInUsers` host+login, no token).
- The token lives in the **macOS Keychain**, service `copilot-cli`,
  account `https://<host>:<login>` — a 41-char **`gho_` OAuth token**.
- Extracting that token and injecting it into the sandbox as
  `COPILOT_GITHUB_TOKEN` makes the in-sandbox `copilot -p` authenticate
  and return a real response, with no PAT involved. **Verified end to end.**

The sandbox already receives `COPILOT_GITHUB_TOKEN` through moor's existing
secret-injection path (the compose template renders it; `resolve_into_env`
fills it). The only missing link was that moor looked for that token only
where an operator *manually* stored it — never at Copilot's own
`copilot-cli` Keychain entry, where `copilot /login` already put a working
one.

## Decision

Copilot auth in the sandbox is the host's **device-flow token, discovered
from the Keychain and injected as `COPILOT_GITHUB_TOKEN`** — not a PAT, and
not a credential the operator hand-copies.

The work splits by single responsibility across the two codebases:

### moor (host side) — discover and inject

`secrets::copilot_device_token()` reads the `copilot-cli` Keychain entry
(`security find-generic-password -s copilot-cli -w` — value on stdout,
never in an argv). For a `copilot`-agent project (the `agent` field
from SPEC-0010, the `--agent` flag), `moor up` resolves it into `COPILOT_GITHUB_TOKEN` **only
when no more explicit source already provided one** — an exported env var
or a moor-managed Keychain secret wins. The decision is a pure
`should_use_device_token(agent_is_copilot, env_set, keychain_set)` so the
priority is tested without touching global state. Nothing is persisted:
the token is read fresh at each `up`, so a rotated/expired device token is
simply re-read, and no copy lands in the manifest, compose file, or a moor
Keychain entry. The value is redacted from the audit trail like every
resolved secret.

### keel (sandbox side) — validate before running

The `copilot` driver preflights the credential it was handed *before*
invoking the agent (or creating a wave worktree): reject a `ghp_` classic
PAT, require a token to be present, and name the exact fix — `copilot
/login` on the host — rather than letting the agent fail opaquely. This is
keel's code (a sibling spec in keel's repo), because keel owns the driver,
the worktree, and reads the token inside the sandbox; moor never sees it.

The seam between them is the one env var moor already injects and the
driver already reads: moor decides *what* token to supply, keel decides
whether it is *usable* before spending a run.

## What this deliberately does not do

- **No PAT path as the happy path.** PATs are not reliably accepted by
  Copilot; the device-flow token is the mechanism. An operator may still
  set `COPILOT_GITHUB_TOKEN` explicitly (it wins), but the seamless path is
  `copilot /login`.
- **No interactive login inside the sandbox.** The flow runs on the host;
  the sandbox receives only the resulting token.
- **No persisted copy of the token.** Read fresh each `up`; moor never
  writes it anywhere, so expiry/rotation needs no moor action.
- **No cross-codebase reach.** moor does not validate the token's
  acceptance (it can't — that needs the live Copilot API); keel does not
  obtain the token (it can't — the Keychain is the host's). Each owns its
  half.
- **Not solved for non-macOS hosts.** Discovery reads the macOS Keychain;
  elsewhere the operator sets `COPILOT_GITHUB_TOKEN` explicitly, and the
  discovery is a silent no-op.

## Consequences

- Copilot in the sandbox becomes "log in once on the host the way you
  already do for other CLIs, then just build" — no token handling, matching
  how the operator uses Kiro.
- **A one-time macOS Keychain prompt is unavoidable.** moor is a different
  binary than the Copilot CLI that created the `copilot-cli` Keychain item,
  so macOS prompts ("Always Allow") the first time moor reads it. One
  click, once, then invisible — but it is not strictly zero-touch, and
  closing it entirely would require code-signing moor into the same
  Keychain access group, which is out of scope here. Documented rather than
  hidden.
- `gho_` device-flow tokens expire; reading fresh each `up` (never caching)
  means an expired token surfaces as the driver's preflight failure the
  next time, pointing the operator back at `copilot /login` — no stale
  copy to hunt down.
- The moor/keel split means neither side has to grow the other's
  knowledge: moor stays the host credential plumber, keel stays the
  in-sandbox gatekeeper, consistent with the whole project's boundary.
