---
id: SPEC-0011
slug: copilot-device-auth
schema: keel.spec/1
status: approved
scope:
- cli/src/secrets.rs
- cli/src/commands/up.rs
- cli/src/compose.rs
budget:
  criteria: 8
  lines: 200
verified_at: 2026-10-01
---

# Resolve the host Copilot device-flow token into the sandbox automatically

## Context

GitHub Copilot CLI does not reliably accept personal access tokens
(classic `ghp_` are rejected; fine-grained usually fail). The mechanism it
*does* accept is its own OAuth device flow: the operator runs
`copilot /login` once on the host, and the CLI stores a `gho_` OAuth token
in the macOS Keychain under service `copilot-cli`, account
`https://<host>:<login>`. Verified locally: extracting that token and
injecting it into the sandbox as `COPILOT_GITHUB_TOKEN` makes the
in-sandbox `copilot -p` authenticate and return a real response, with no
PAT involved.

moor already resolves a project's declared secrets at `up` (shell env,
then its own Keychain entries — `secrets::resolve_into_env`) and injects
them into the sandbox. The one missing link is that `COPILOT_GITHUB_TOKEN`
is only found if the operator *manually* stored a token; moor does not yet
look at the Copilot CLI's own `copilot-cli` Keychain entry, which is where
`copilot /login` already put a working token. Closing that gap makes
Copilot auth seamless: log in once on the host the way you already do for
other CLIs, and the sandbox is authenticated with nothing token-shaped to
set.

Single responsibility: this spec owns the **host side** — discovering the
device-flow token and injecting it. The in-sandbox driver's preflight and
error messaging are keel's, specified separately.

## Acceptance criteria

### AC-1 The Copilot CLI's device-flow token is discoverable from the host Keychain

WHEN a host has completed `copilot /login` THE SYSTEM SHALL read the
resulting token from the macOS Keychain service `copilot-cli` without the
operator having stored it via `moor secrets`.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml finds_copilot_device_token_source` exit 0

### AC-2 An explicitly set credential always wins over the discovered token

WHEN `COPILOT_GITHUB_TOKEN` is already resolvable from the shell env or the
project's own Keychain entry THE SYSTEM SHALL use that and SHALL NOT
override it with the discovered device-flow token.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml explicit_credential_wins_over_device_token` exit 0

### AC-3 The device-flow token is only used for copilot-agent projects

WHERE the project's agent is not `copilot` THE SYSTEM SHALL NOT inject the
discovered device-flow token.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml device_token_only_for_copilot_agent` exit 0

### AC-4 The token is read fresh each up, never persisted by moor

WHEN moor injects the discovered device-flow token THE SYSTEM SHALL read it
at resolution time and SHALL NOT write a copy into the project's manifest,
compose file, or its own Keychain entries.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml device_token_not_persisted_by_moor` exit 0

### AC-5 The token never passes through a process argument

WHEN the discovered token is injected into the sandbox THE SYSTEM SHALL
pass it by environment or stdin, never as a command-line argument, so it
cannot appear in `docker inspect`/`ps` output.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml device_token_not_in_argv` exit 0

### AC-6 Absence of a device-flow token is a no-op, not an error

IF no `copilot-cli` Keychain entry exists THEN THE SYSTEM SHALL continue
without error, leaving `COPILOT_GITHUB_TOKEN` unresolved for the driver's
own preflight to report.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml missing_device_token_is_noop` exit 0

### AC-7 The resolved token is scrubbed from the audit trail

WHEN the discovered token is injected THE SYSTEM SHALL redact its value
from logged exec records the same way every resolved secret is.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml device_token_is_redacted` exit 0

### AC-8 Existing CLI tests stay green

THE SYSTEM SHALL leave the existing CLI test suite passing.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml` exit 0

## Out of scope

- The in-sandbox driver's credential preflight, failure messaging, and
  classic-PAT rejection — those are keel's (sibling spec
  `copilot-device-flow-preflight`).
- Running the device flow itself: the operator runs `copilot /login` on the
  host; moor never performs an interactive login.
- Non-macOS hosts: the discovery reads the macOS Keychain; on other hosts
  the operator sets `COPILOT_GITHUB_TOKEN` explicitly (AC-6's no-op path).
- Token refresh/expiry handling beyond reading fresh at each `up` — a
  device-flow token that has expired is the driver's preflight to report.
