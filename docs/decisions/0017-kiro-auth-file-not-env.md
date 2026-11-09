# ADR-0017: kiro-cli reads its login from a credential file, not an env var

**Status:** Proposed
**Date:** 2026-11-09

## Context

A real `sc-analyze` (kiro-agent) project could not run `moor go`: every
build turn blocked before any gate executed. Diagnosis on the live sandbox
found **two independent defects**. The first is fixed; the second is what
this ADR is about.

### 1. kiro-cli's session database had no writable backing (fixed, v0.10.4)

kiro-cli opens a SQLite session store at
`~/.local/share/kiro-cli/data.sqlite3`. The sandbox rootfs is read-only, and
the compose template mounted a writable named volume for kiro's credential
dir (`~/.aws`) but **not** for the session-DB path. So the turn authenticated
far enough to call the agent, then blocked with:

```
kiro-cli exited non-zero: error: Failed to open database: timed out waiting
for connection: unable to open database file:
/home/agent/.local/share/kiro-cli/data.sqlite3
```

after a ~60s SQLite lock wait. Fixed in v0.10.4 by adding a `kiro-data` named
volume at `~/.local/share/kiro-cli` (PR #90). Confirmed on the live sandbox:
`data.sqlite3` now exists and the path is `rw`. **This ADR does not revisit
that fix** — it is recorded here only so the two defects are not conflated.

### 2. moor injects the SSO token as an env var; kiro-cli wants a file

With the DB fix in place, the turn got further and then blocked in ~0.2s with:

```
invalid driver result — stdout is not JSON: control character
(\u0000-\u001F) found while parsing a string at line 1 column 112
```

The control character is an ANSI escape (`\u001B[?25l`, a cursor-hide plus a
progress spinner). kiro-cli, finding no login, tried to **open a browser to
authenticate**, could not (headless sandbox), and wrote that spinner to
stdout before exiting non-zero. The keel kiro driver wraps the tool's output
with `keel_emit`, whose sed escaping covers `"` and `\` but **not** raw
control characters — so the escape leaked into the JSON string and broke the
parse. The JSON-parse error is a *symptom*; the cause is that kiro-cli is not
authenticated inside the sandbox.

#### Why it is not authenticated — the version/auth-source mismatch

moor's kiro credential path (SPEC-0018 / `secrets::resolve_kiro_credential`)
reads the host's freshest unexpired SSO access token from
`~/.aws/sso/cache/*.json` and injects it into the sandbox as the **env var**
`AWS_BEARER_TOKEN_BEDROCK`. That was verified against **kiro-cli 2.19.1** (the
driver's own header says so).

Observed on 2026-11-09:

| Where | kiro-cli version | How it authenticates |
| --- | --- | --- |
| moor driver assumes | 2.19.1 | `AWS_BEARER_TOKEN_BEDROCK` env var |
| sandbox image (`KEEL_VERSION`/image pin) | 2.37.1 | a credential **file** |
| host (`brew`/installer) | 3.1.0 | `~/.aws/sso/cache/kiro-auth-token.json` |

Evidence that current kiro-cli reads a **file**, not the env var:

- On the **host**, kiro-cli 3.1.0 is logged in (`whoami` succeeds). Its only
  backing file is `~/.aws/sso/cache/kiro-auth-token.json`. There is no
  `~/.local/share/kiro-cli/` or `~/.config/kiro-cli/` credential on the host.
- In the **sandbox**, `AWS_BEARER_TOKEN_BEDROCK` **is** set (the injected
  token, length 232, and confirmed to be the value from the host's
  `kiro-auth-token.json`). Yet `kiro-cli whoami` returns "Not logged in" —
  **with and without `AWS_REGION=eu-west-1` also set**. The env var is not
  consumed as a login.
- The sandbox has **no `~/.aws/sso/cache/` directory at all**, so there is no
  `kiro-auth-token.json` for kiro-cli to read. Hence the browser fallback.

So moor injects the token's *string* as an env var, but kiro-cli (2.37.1+)
reads its login from the `kiro-auth-token.json` *file* in the SSO cache, which
moor never populates inside the sandbox. The env-var approach that worked on
2.19.1 is no longer a supported login mode.

### Update (2026-11-09): the token-file hypothesis was tested and is WRONG

The "inject the `kiro-auth-token.json` file" idea below was tested directly
and **does not authenticate**. Streaming the host's `kiro-auth-token.json`
*and* `kiro-connection.json` into the sandbox's `~/.aws/sso/cache/` still left
`kiro-cli whoami` saying "Not logged in". The reason: kiro-cli 2.37.1 keeps
its login in its own **`~/.kiro` store** (a `data.sqlite3` plus an
`enterprise-management/` client registration), not in a portable SSO token
file. On the host, `~/.kiro` is where the identity lives; in the sandbox
`~/.kiro` is **empty and on the read-only rootfs** (only
`~/.local/share/kiro-cli` has a writable volume).

So headless kiro auth is **not** reconstructable by copying token files, and
the real contract for 2.37.1+ is still unknown. Open questions a correct fix
must answer first:

1. Does kiro-cli 2.37.1+ support **any** non-interactive auth (a device-flow
   token, or an `AWS_*`/`KIRO_*` bearer mode this version actually honours),
   or is interactive `kiro-cli login` the only path?
2. If it needs `~/.kiro` state, is that state portable host→sandbox at all, or
   machine-bound (client registration)? Would a writable `~/.kiro` volume plus
   a one-time in-sandbox `kiro-cli login --use-device-flow` be the model?
3. What exact version should the image pin to, re-verified against whichever
   of the above is true?

Until those are answered, the credential-injection decision below is
**deferred** — do not implement it as written (file injection is disproven).


## Decision (proposed — needs review before implementing)

**Make moor place the host's kiro credential into the sandbox as the file
kiro-cli actually reads, rather than (or in addition to) an env var — and pin
the kiro-cli version so the auth contract cannot drift silently again.**

Concretely, for review:

1. **File-based credential injection.** At `moor up` for a kiro project, write
   the host's current `~/.aws/sso/cache/kiro-auth-token.json` (and, if
   required, `kiro-connection.json`) into the sandbox's `~/.aws/sso/cache/`
   on the existing `kiro-state` volume — read fresh from the host each `up`,
   never baked into the image, mirroring how the token is read fresh today.
   The exact filename and whether `kiro-connection.json` is also required must
   be confirmed against the pinned kiro-cli version, not assumed.
2. **Keep the no-host-bind-mount invariant.** The credential must cross the
   same way the SSO token string does now — read on the host, written into a
   named volume via moor — **not** a host bind mount of `~/.aws`. The
   `compose.rs` no-bind-mount test stays authoritative.
3. **Pin kiro-cli in the image.** The driver was verified against 2.19.1, the
   image ships 2.37.1, the host runs 3.1.0. Pin the sandbox's kiro-cli version
   (as `KEEL_VERSION` is pinned) so a future auth change is a deliberate bump
   with a re-verification, not a silent break.
4. **Harden `keel_emit` against control characters. (IMPLEMENTED — see below.)**
   Independently of auth, the kiro driver must never emit non-JSON:
   `keel_emit` should strip or `\u`-escape control characters (as
   `guide::printable` already does for sandbox text), so a tool that prints
   ANSI on failure yields a *blocked* result with a readable detail instead of
   an unparseable one. This is a defence-in-depth fix: it turns the opaque
   "stdout is not JSON" into the real "kiro-cli is not authenticated" message.

### Implemented in this change: the `keel_emit` control-char guard (step 4)

`.keel/drivers/_common.sh`'s `keel_emit` now deletes the whole C0 control
range (`tr -d '\000-\037'`) from `detail`, not just newlines. Verified: a
`detail` carrying an ESC + ANSI spinner + carriage return (exactly kiro-cli's
failed-login output) now serialises to valid JSON. Confirmed end to end on the
live `sc-analyze` sandbox — the run that previously blocked with
`invalid driver result — stdout is not JSON: control character … column 112`
now blocks with `kiro-cli is not authenticated: … Failed to open browser …
Run kiro-cli login …`, i.e. the true reason and remedy.

This is the only part of the decision that is safe to ship now: it changes
only moor's own driver escaping (keel owns the quoting), fixes nothing about
auth, and strictly improves every driver's failure legibility. The auth
injection (steps 1–3) stays deferred pending the open questions above.

## Why these choices

- **Match the tool's real contract.** The env-var path is a dead end on
  current kiro-cli; the file is what `whoami`/`chat` actually read. Injecting
  the file is the smallest change that restores headless auth.
- **Read fresh, never persist.** SPEC-0018's security properties (read fresh
  each `up`, no copy in the manifest/compose/Keychain, never in an argv) must
  hold for the file too. Writing it into a named volume at `up` from a
  freshly-read host file preserves "the credential lives on the Mac" — the
  volume copy is disposable and recreated each `up`.
- **Pin to stop the drift.** The root cause is an unpinned agent CLI whose
  auth changed under moor. Pinning makes the adapter's "verified against
  <version>" claim true and testable.
- **Escape control chars regardless.** Even once auth works, a driver that
  can emit raw ANSI on any error is one bad exit away from the same opaque
  failure. Stripping control characters in `keel_emit` is cheap and makes
  every driver's failure legible.

## What this deliberately does not do

- **It does not keep the env-var injection as the primary mechanism.** It may
  remain as a fallback if a kiro-cli version still honours it, but the file is
  the supported path.
- **It does not introduce a host bind mount.** The credential crosses via a
  named volume written by moor, as the token string does today.
- **It does not solve Builder ID / social login.** As ADR-0017's predecessor
  work noted, the SSO-cache path is Identity Center; a free-tier login may
  store its credential elsewhere and is out of scope here.
- **It does not change keel's driverresult schema** — only moor's own
  `keel_emit` escaping, which already owns the quoting.

## Consequences

- A kiro project authenticates headlessly again: moor places the credential
  file kiro-cli reads, so `moor go` drives the agent instead of falling back
  to a browser login that cannot run.
- A driver that fails with ANSI output reports *why* (an auth/credential
  message) rather than an opaque JSON-parse error.
- The sandbox's kiro-cli version is pinned, so the auth contract is a
  reviewed, testable assumption rather than whatever the installer served that
  day.
- Status is **Proposed**: this needs a spec and review before implementation,
  because it changes how a credential crosses into the sandbox — the same
  security-sensitive boundary SPEC-0018 and ADR-0009 govern.

## Reproduction (for whoever implements this)

On a kiro project whose host has a valid `kiro-cli login`:

```
moor up <project>
moor run <project> -- sh -c 'echo $AWS_BEARER_TOKEN_BEDROCK | head -c 8; kiro-cli whoami'
#   token is set, whoami says "Not logged in"
moor run <project> -- sh -c 'ls ~/.aws/sso/cache/ 2>&1'
#   No such file or directory  ← the credential file kiro-cli wants is absent
moor run <project> -- sh -c 'kiro-cli --version'   # 2.37.1, not the driver's 2.19.1
```
