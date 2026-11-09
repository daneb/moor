# ADR-0018: kiro authenticates via `KIRO_API_KEY`, like every other agent

**Status:** Accepted
**Date:** 2026-11-09
**Supersedes:** the auth decision in ADR-0017 (its `keel_emit` control-char
guard stands); supersedes SPEC-0018 acceptance criteria AC-6, AC-7, AC-8, AC-9,
AC-10 (the SSO-cache credential story).

## Context

moor's kiro support (SPEC-0018) read the host's IAM Identity Center SSO token
from `~/.aws/sso/cache/*.json` and injected it into the sandbox as the env var
`AWS_BEARER_TOKEN_BEDROCK`. ADR-0017 recorded that this no longer works:
kiro-cli 2.37.1+ **ignores** that env var for login, and the token/file is not
a portable credential (the identity is a machine-bound `~/.kiro` store). Two
candidate fixes in ADR-0017 — env-var bearer token, then file injection — were
each **disproven by live test**.

Investigation then established, from kiro-cli's own output and the vendor docs,
that current kiro-cli has exactly two auth modes:

1. **`KIRO_API_KEY`** — an API key from the Kiro portal, set as an env var;
   per kiro.dev, "that single environment variable is the difference between
   interactive and headless mode." Proven in the sandbox: with `KIRO_API_KEY`
   set and a writable `~/.kiro`, kiro-cli skipped the browser entirely and
   reached a real API dispatch (the dummy key was then rejected at the service,
   as expected).
2. **`kiro-cli login`** — interactive, writing to the machine-bound `~/.kiro`
   store; cannot complete headlessly in the sandbox.

`AMAZON_Q_SIGV4` (an Amazon Q feature) was tested and is **gone** from
kiro-cli 2.37.1 — the CLI responded "Not logged in. Set the KIRO_API_KEY
environment variable or run `kiro-cli login` first," naming only the two modes
above.

This is the same shape every other agent already uses: Claude reads
`ANTHROPIC_API_KEY`/`CLAUDE_CODE_OAUTH_TOKEN`, Copilot reads
`COPILOT_GITHUB_TOKEN` — an env-var credential moor's secret→env plumbing
injects. kiro was the outlier, built on a bespoke SSO-cache reader that the
product has since obsoleted.

## Decision

**A kiro project authenticates from `KIRO_API_KEY`, declared as an ordinary
project secret and resolved by the normal env/Keychain path — exactly like
`ANTHROPIC_API_KEY`. The SSO-cache machinery is removed.**

- **`manifest::KIRO_SECRET_VARS` becomes `["KIRO_API_KEY"]`** (was the three
  `AWS_*`/`KIRO_START_URL` names). `declare_agent_secrets` still adds it only
  for a kiro project, so a non-kiro manifest never carries it.
- **`secrets.rs` loses the entire SSO path** — `KIRO_TOKEN_VAR`,
  `kiro_cache_dir`, `kiro_sso_token`, `token_from_cache_file`,
  `resolve_kiro_credential`, `should_use_kiro_token`, and their five SPEC-0018
  tests. No bespoke kiro resolver replaces them: `compose_up`'s existing
  `resolve_into_env` carries `KIRO_API_KEY` from the operator's env or moor's
  Keychain, the same as every other declared secret.
- **`up.rs` loses its `resolve_kiro_credential` call.** The Copilot
  device-token resolver stays (Copilot genuinely needs a reach into its CLI's
  own Keychain service; kiro does not — a Kiro API key is an explicit value
  the operator holds).
- **The kiro auth hint changes** to: store a key with
  `moor secrets set <project> KIRO_API_KEY` (an admin may need to enable
  API-key access for the org), then `moor up`.
- **The kiro image comment** is corrected to the API-key model.

## Why not mirror Copilot's Keychain reader

Copilot's `copilot_device_token` reaches into a *third-party* Keychain service
(`copilot-cli`) that the Copilot CLI writes and moor does not own — moor must
read it out because the operator never sees that token. `KIRO_API_KEY` is the
opposite: an explicit key the operator obtains and stores deliberately. The
correct analogue is therefore `ANTHROPIC_API_KEY` (a declared secret via
`resolve_into_env`), which is also strictly less code and less host-coupling.

## Consequences

- A kiro project runs headlessly once `KIRO_API_KEY` is set — the same
  one-secret setup as Claude. `moor secrets status` already reports whether it
  resolves from env, Keychain, or is missing.
- moor no longer reads `~/.aws/sso/cache` at all; the three-version drift
  (driver 2.19.1 / image 2.37.1 / host 3.1.0) that made the SSO path fail
  silently is gone, because the API key is version-independent.
- **The honest limitation:** `KIRO_API_KEY` may be **admin-disabled** for an
  enterprise IAM Identity Center org (Kiro console → Settings → Kiro settings →
  *Enable users to generate API keys*), and key generation for external IdPs
  has an open upstream bug ([kirodotdev/Kiro#8414]). An operator in that
  situation cannot run a kiro project headlessly in moor at all; the documented
  workaround is to use a different agent (Claude or Copilot) for that project,
  which moor supports by editing the manifest's `image`/`agent`. moor does not
  attempt an in-sandbox interactive login (it cannot complete headlessly, and
  it would persist a machine-bound credential inside the sandbox — a departure
  from moor's host-only-credential model this ADR deliberately declines).

## What this deliberately does not do

- **It does not add an in-sandbox `kiro-cli login` path.** Device-flow login
  into a persistent `~/.kiro` volume was considered and rejected: it is
  interactive, and it persists a credential inside the sandbox, which breaks
  the "credential lives on the host, injected fresh" principle (ADR-0009).
- **It does not change the compose volumes.** The `kiro-data` volume
  (v0.10.4) for kiro-cli's SQLite session DB stays; it is unrelated to auth.
  The `kiro-state`→`~/.aws` volume is now vestigial for auth but harmless;
  removing it is a separate cleanup.
- **It does not change the keel driver** beyond ADR-0017's shipped
  control-char guard.

## SPEC-0018 criteria status

- **AC-5** (a kiro project records the credential env name as a secret): still
  holds — the recorded name is now `KIRO_API_KEY`.
- **AC-6** (host credential auto-discovered from the SSO cache, no `moor
  secrets` step): **superseded.** There is no auto-discovery; the operator
  stores the key explicitly, like `ANTHROPIC_API_KEY`.
- **AC-7** (credential only injected for kiro projects): still true in effect —
  only a kiro manifest declares `KIRO_API_KEY`, so only a kiro project resolves
  it — but the dedicated injector/test is gone.
- **AC-8** (read fresh, never persisted by moor): **superseded.** `KIRO_API_KEY`
  is deliberately stored in moor's Keychain (`moor secrets set`), the same as
  any API key.
- **AC-9** (never in an argv): still true via `resolve_into_env`/compose
  `${VAR}` injection, the same guarantee every secret has.
- **AC-10** (absence is a no-op): still true — an unset secret resolves to
  empty; the turn's auth hint reports the gap.

[kirodotdev/Kiro#8414]: https://github.com/kirodotdev/Kiro/issues/8414
