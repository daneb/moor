---
id: SPEC-0016
slug: git-push-on-host
schema: keel.spec/1
status: approved
scope:
  - "cli/src/commands/ship_cmd.rs"
  - "cli/src/commands/run_cmd.rs"
  - "cli/src/commands/import_cmd.rs"
  - "cli/src/commands/new_cmd.rs"
  - "cli/src/commands/pull_cmd.rs"
  - "cli/src/commands/mod.rs"
  - "cli/src/compose.rs"
  - "cli/src/manifest.rs"
  - "cli/src/proc.rs"
  - "cli/src/main.rs"
  - "cli/templates/**"
  - "docs/decisions/0014-*.md"
budget:
  criteria: 11
  lines: 680
verified_at: 2026-10-14
---

# Move credentialed git to the host so the GitHub token never enters the sandbox

## Context

The sandbox is the agent's playroom for potentially dubious behaviour, yet
today it is handed a live GitHub credential. `cli/src/compose.rs` injects
`GITHUB_TOKEN: ${GITHUB_TOKEN:-}` into the container env (declared as a
sandbox secret in `manifest.rs`), and the same in-container credential
helper — `!f() { echo username=x-access-token; echo
"password=$GITHUB_TOKEN"; }; f` — reads it in **four** places:

- `ship_cmd.rs` pushes a shipped branch from the sandbox.
- `new_cmd.rs` clones a freshly created private repo into the sandbox.
- `import_cmd.rs` imports an existing repo's history into the sandbox.
- `pull_cmd.rs` pulls the sandbox's branch over HTTPS.

So a compromised or manipulated agent run — the exact risk category moor
exists to contain — shares an environment with a token that can push to
and read from the operator's GitHub. The README already lists the token's
exposure among the open security concerns; this closes it, across every
site rather than just one, so stripping the token from the sandbox env is
actually true and not a half-move.

ADR-0010 (no `ssh` in the sandbox) is why `pull` rewrites `origin` to an
HTTPS URL today; running the fetch/pull on the host removes that
constraint — the host has `ssh` and can use the native remote directly.

Two things make this a clean move rather than a rewrite:

- **The PR half is already on the host.** `ship_cmd.rs` runs `gh pr
  create` via `proc::run_capture("gh", …)` on the host today, so a
  host-side GitHub credential is already assumed to exist. This change
  makes `push` depend on that same host credential instead of on an
  in-container one — it does not introduce a new host dependency.
- **The transport already exists in the opposite direction.**
  `import_cmd::import_history` streams a `git bundle` *into* a fresh
  sandbox over `docker exec -i … "cat > /tmp/import.bundle"`, and
  documents why `docker cp` cannot be used (the read-only rootfs defeats
  it even onto a writable tmpfs mount). Shipping is the mirror: stream a
  `git bundle` *out* over `docker exec … git bundle create -` to host
  stdout, then let the host push it with its own token.

The design (confirmed with the operator):

1. The commit is still made **inside** the sandbox — local git needs no
   credential, and keeping it there preserves ADR-0007's synced authorship
   untouched.
2. The host extracts just that branch's commits as a `git bundle` stream
   over `docker exec` (no bind mount, read-only rootfs intact), fetches
   them into an **ephemeral** scratch repo, and pushes to GitHub with the
   host's own credential. The host keeps no standing clone.
3. `GITHUB_TOKEN` is removed from the sandbox compose env entirely.
4. Private-repo clone (`import`/`run`) likewise fetches on the host and
   streams history *in*, so clone no longer needs an in-container token.
5. If the host has no usable credential, the flow fails loudly before it
   commits anything and never falls back to the in-sandbox token path.

`moor ship` stays the single entry point; its contract to the operator is
unchanged — only where the credential lives changes. A new ADR records the
posture shift, matching the repo's practice (ADR-0002 injected agent auth,
ADR-0007 synced git identity).

## Acceptance criteria

### AC-1 The sandbox env carries no GitHub token

WHEN the compose template for a sandbox is rendered THE SYSTEM SHALL omit
any `GITHUB_TOKEN` environment entry from the sandbox service.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet compose::tests::sandbox_env_has_no_github_token` exit 0

### AC-2 No in-sandbox git credential helper remains in ship

WHEN the shipping code is inspected THE SYSTEM SHALL contain no credential
helper that reads `$GITHUB_TOKEN` inside the container.

oracle: cmd `test -z "$(grep -n 'password=\$GITHUB_TOKEN' cli/src/commands/ship_cmd.rs)"` exit 0

### AC-3 The commit is still made inside the sandbox

WHEN `moor ship` commits a spec's work THE SYSTEM SHALL create the commit
with git executed inside the sandbox, so the committed author remains the
ADR-0007 synced identity.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet ship_cmd::tests::commit_runs_in_sandbox` exit 0

### AC-4 The branch's commits leave the sandbox as a bundle stream

WHEN the host needs the sandbox's new commits to push THE SYSTEM SHALL
obtain them by streaming a `git bundle` out over `docker exec` rather than
through any bind mount or `docker cp`.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet ship_cmd::tests::commits_leave_via_bundle_stream` exit 0

### AC-5 The push runs on the host with the host credential

WHEN a spec is shipped to a GitHub remote THE SYSTEM SHALL run `git push`
from the host authenticated by the host's own credential, not by any
value present in the sandbox.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet ship_cmd::tests::push_runs_on_host` exit 0

### AC-6 A missing host credential fails before anything is committed

IF the host has no usable GitHub credential WHEN `moor ship` is invoked
THEN THE SYSTEM SHALL stop and name the host-side fix before creating a
branch or commit, and SHALL NOT push from inside the sandbox.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet ship_cmd::tests::absent_host_credential_fails_before_commit` exit 0

### AC-7 The host-side push is recorded with its branch and commit

WHEN the host completes a push for a shipped spec THE SYSTEM SHALL append a
`push` entry to the project's audit chain carrying the branch ref and the
exact commit sha, as the in-sandbox push did.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet ship_cmd::tests::host_push_is_recorded_with_ref_and_commit` exit 0

### AC-8 Creating a project clones its repo from the host

WHEN `moor new --github` populates a new private repo's workspace THE
SYSTEM SHALL clone it with the host credential and stream the result into
the sandbox, using no in-container credential helper.

oracle: cmd `test -z "$(grep -n 'password=\$GITHUB_TOKEN' cli/src/commands/new_cmd.rs)"` exit 0
oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet new_cmd::tests::github_clone_uses_host_credential` exit 0

### AC-9 Pulling a sandbox branch runs on the host

WHEN `moor pull` refreshes the sandbox's branch from GitHub THE SYSTEM
SHALL fetch on the host with the host credential and apply the result
into the sandbox, using no in-container credential helper.

oracle: cmd `test -z "$(grep -n 'password=\$GITHUB_TOKEN' cli/src/commands/pull_cmd.rs)"` exit 0
oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet pull_cmd::tests::pull_runs_on_host` exit 0

### AC-10 No in-container GitHub-token credential helper remains anywhere

WHEN the CLI source is inspected THE SYSTEM SHALL contain no credential
helper that reads `$GITHUB_TOKEN` inside the container in any command.

oracle: cmd `test -z "$(grep -rn 'password=\$GITHUB_TOKEN' cli/src)"` exit 0

### AC-11 The posture change is recorded as an ADR, and the suite stays green

WHEN this change lands THE SYSTEM SHALL add an ADR explaining that
credentialed git executes on the host and the token never enters the
sandbox, and SHALL leave the existing CLI and MCP test suites passing.

oracle: cmd `test -f docs/decisions/0014-git-on-host.md` exit 0
oracle: cmd `cargo test --quiet --manifest-path cli/Cargo.toml && cargo test --quiet --manifest-path mcp/Cargo.toml` exit 0

## Out of scope

- **Interactive `moor shell` git.** An operator who opens a shell and runs
  `git push` by hand is outside this flow; the threat model already notes
  in-shell commands aren't mediated the way `moor run` is. This spec moves
  moor's *own* credentialed git out, not every possible manual invocation.
- **Changing commit authorship or ADR-0007.** The commit stays in-sandbox
  precisely so synced identity is untouched; re-staging the agent's files
  to author the commit on the host is a different design that is not taken.
- **`gh pr create` behaviour.** It already runs on the host; its
  degraded-path messaging is unchanged beyond the new fail-loud credential
  check that precedes it.
- **Removing non-git host dependencies or re-architecting the egress
  allowlist.** With pushes leaving from the host, the sandbox's egress
  allowlist no longer needs github.com for moor's own git — but tightening
  the allowlist is a follow-up, not part of this move.
