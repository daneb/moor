---
id: SPEC-0018
slug: kiro-agent-sandbox
schema: keel.spec/1
status: approved
scope:
- cli/src/manifest.rs
- cli/src/commands/new_cmd.rs
- cli/src/commands/import_cmd.rs
- cli/src/main.rs
- cli/src/secrets.rs
- cli/src/commands/up.rs
- cli/src/compose.rs
- cli/templates/project.compose.yml.tmpl
- images/kiro/Dockerfile
- images/build.sh
- proxy/allowlist.base.txt
budget:
  criteria: 12
  lines: 460
---

# Run a project's sandbox on the Kiro CLI agent

## Context

moor's turn protocol already speaks Kiro. `Agent::Kiro` parses from
`--agent kiro`; `session::profile(Agent::Kiro)` invokes `kiro-cli chat
--no-interactive`, trusts tools with `--trust-tools`, reads plain-text
output, and prints a Kiro-specific auth hint; `refuse_unrestrainable_role`
correctly refuses a tool-restricting role on Kiro (it has only an
auto-approval flag, no deny flag — the same ADR-0008 limitation). All of
that is unit-tested. What is missing is everything *below* the turn: the
sandbox a Kiro turn runs inside.

Three gaps keep a Kiro project from actually running:

1. **No image carries the binary.** `manifest::resolve_image` only composes
   the Copilot layer; for any non-Copilot agent it returns the language
   image unchanged. A Kiro project therefore lands on `moor/base|node|rust|
   python`, none of which install `kiro-cli` — the turn fails at `exec` with
   "kiro-cli: not found". Copilot solved this with an opt-in
   `images/copilot/Dockerfile` layer composed as `moor/copilot-<lang>`;
   Kiro needs the same shape (`images/kiro/Dockerfile` → `moor/kiro-<lang>`).

2. **No credential reaches the sandbox.** Kiro's enterprise auth is IAM
   Identity Center via the OAuth device flow (`kiro-cli login --license pro
   --identity-provider <start-url> --region <region>`, verified against
   kiro-cli 2.34.2). Unlike Copilot — a single `COPILOT_GITHUB_TOKEN` env
   var — the host credential is a set of cached SSO tokens under
   `~/.aws/sso/cache` plus kiro-cli's own store, and the CLI reads a family
   of `KIRO_*`/`AWS_*` environment variables. moor's default manifest
   secrets name none of these, and `up`/`secrets` inject none of them. This
   is the genuinely new design work — a single env token is not sufficient.

3. **The egress allowlist blocks the auth and model calls.** The
   default-deny proxy (`proxy/allowlist.base.txt`) permits GitHub,
   Anthropic, and Copilot hosts only. A Kiro turn reaches AWS SSO OIDC, AWS
   CodeWhisperer (the CodeWhisperer ARN in `kiro-cli whoami` confirms the
   service), and the operator's IDC start-URL host — all currently denied,
   so even an authenticated sandbox cannot complete a turn.

This spec owns the **sandbox layer for the Kiro agent**: image, image
composition, credential injection, egress, and the operator-facing flag/
help surface. It does not touch the already-built turn protocol beyond the
image-reconciliation accessor. The credential is delivered into the sandbox
by a `kiro-state` volume mounted at `~/.aws` in the project compose
template (AC-8/AC-9's file-not-argv requirement), mirroring the existing
`claude-state`/`copilot-state` volumes there.

Precedent to mirror exactly: SPEC-0010 (agent-selection) for the image
axis, SPEC-0011 (copilot-device-auth) for host-credential-into-sandbox, and
`images/copilot/Dockerfile` for an opt-in agent layer.

## Acceptance criteria

### AC-1 Kiro composes with the detected language into a kiro image

WHEN the agent is `kiro` AND the language image is `moor/<lang>:latest`
THE SYSTEM SHALL resolve the project image to `moor/kiro-<lang>:latest`, and
to `moor/kiro:latest` when the language is base.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml kiro_composes_with_language` exit 0

### AC-2 An already-kiro image is left unchanged by resolution

WHEN the agent is `kiro` AND the image is already a `moor/kiro` or
`moor/kiro-<lang>` image THE SYSTEM SHALL return that image unchanged rather
than composing a second kiro prefix.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml kiro_image_resolution_is_idempotent` exit 0

### AC-3 A kiro image reconciles a manifest whose agent field is stale

WHEN a manifest's image is a `moor/kiro` or `moor/kiro-<lang>` image AND its
`agent` field is absent or disagrees THE SYSTEM SHALL report the reconciled
agent as `kiro`, the same way a copilot image reconciles to copilot.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml kiro_image_reconciles_agent` exit 0

### AC-4 The --agent flag help names kiro as a valid value

WHEN the operator reads `moor new --help` or `moor import --help` THE SYSTEM
SHALL list `kiro` among the accepted `--agent` values.

oracle: cmd `cargo run --manifest-path cli/Cargo.toml -- new --help 2>&1 | grep -q kiro` exit 0

### AC-5 A kiro-agent project records the Kiro credential env names as secrets

WHEN a project is created with `--agent kiro` THE SYSTEM SHALL record in its
manifest the environment-variable names Kiro's sandbox credential is carried
in, and SHALL NOT record them for a non-kiro project.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml kiro_project_declares_kiro_secrets` exit 0

### AC-6 The host Kiro credential is discoverable without manual secret entry

WHEN a host has completed `kiro-cli login` for IAM Identity Center THE
SYSTEM SHALL locate the resulting cached credential on the host without the
operator having stored anything via `moor secrets`.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml finds_kiro_credential_source` exit 0

### AC-7 The Kiro credential is only injected for kiro-agent projects

WHERE a project's reconciled agent is not `kiro` THE SYSTEM SHALL NOT inject
the discovered Kiro credential into its sandbox.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml kiro_credential_only_for_kiro_agent` exit 0

### AC-8 The Kiro credential is read fresh each up and never persisted by moor

WHEN moor injects the discovered Kiro credential THE SYSTEM SHALL read it at
resolution time and SHALL NOT write a copy into the manifest, the compose
file, or its own Keychain entries.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml kiro_credential_not_persisted` exit 0

### AC-9 The Kiro credential never passes through a process argument

WHEN the discovered Kiro credential is injected into the sandbox THE SYSTEM
SHALL pass it by environment or file, never as a command-line argument, so
it cannot appear in `docker inspect` or `ps` output.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml kiro_credential_not_in_argv` exit 0

### AC-10 Absence of a Kiro credential is a no-op, not an error

IF no host Kiro credential is present THEN THE SYSTEM SHALL continue without
error, leaving the credential unresolved for the turn's own auth hint to
report.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml missing_kiro_credential_is_noop` exit 0

### AC-11 The egress allowlist permits Kiro's auth and model endpoints

THE SYSTEM SHALL include in the baseline egress allowlist the AWS SSO OIDC
and AWS CodeWhisperer host patterns Kiro's device-flow login and chat turns
require, so a kiro-agent sandbox can authenticate and complete a turn
through the default-deny proxy.

oracle: cmd `grep -Eq 'codewhisperer|sso|oidc' proxy/allowlist.base.txt && echo ok` exit 0

### AC-12 The whole CLI and image-compose test suite stays green

THE SYSTEM SHALL leave the existing `cli` test suite passing.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml` exit 0

## Out of scope

- **The turn protocol itself.** `session::profile(Agent::Kiro)`,
  `build_turn_argv`, `refuse_unrestrainable_role`, `failure_hint`, and their
  tests are already implemented and verified; this spec changes none of
  them except through the image-reconciliation accessor (AC-3).
- **Running the device flow.** The operator runs `kiro-cli login` on the
  host; moor never performs an interactive login. Token refresh and expiry
  beyond reading fresh at each `up` are the turn's auth hint to report
  (AC-10), not this spec's.
- **The `.keel/drivers/kiro` driver.** That script (in the keel repo, for
  `keel run --driver kiro`) is a separate mechanism from moor's agent turn
  and is not in scope here.
- **Serena and other host MCP side-effects.** The host kiro-cli's MCP
  servers (Serena, Playwright, …) that write into the working tree are a
  host-interactive concern; inside moor's sandbox the agent gets only the
  image's tool surface. Pruning the host MCP set is not this spec's work.
- **Non-macOS hosts.** Credential discovery targets the macOS host store;
  on other hosts the operator sets the credential env names explicitly,
  which AC-10's no-op path already accommodates.
- **Building every `moor/kiro-<lang>` combination in CI.** `images/build.sh`
  gains the kiro layer (AC-1 depends on the image existing locally), but the
  CI matrix that scans every combined image is a follow-on, exactly as
  SPEC-0010 deferred the copilot combinations.
