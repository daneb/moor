# ADR-0014: credentialed git runs on the host, never in the sandbox

**Status:** Accepted
**Date:** 2026-10-14

## Context

The sandbox is where moor lets an agent do its potentially dubious work,
yet it was handed a live GitHub credential. `compose.rs` injected
`GITHUB_TOKEN: ${GITHUB_TOKEN:-}` into the sandbox's environment
(declared as a secret in `manifest.rs`), and an in-container git
credential helper — `!f() { echo username=x-access-token; echo
"password=$GITHUB_TOKEN"; }; f` — read it in four places: `moor ship`'s
push, `moor new --github`'s clone of a freshly created private repo,
`moor import`'s clone of an existing remote, and `moor pull`'s refresh of
the sandbox's branch. A compromised or manipulated agent run — the exact
risk category moor exists to contain — shared an environment with a
token that could push to and read from the operator's GitHub.

Two things made moving this out a clean change rather than a rewrite:

- `moor ship` already ran `gh pr create` on the host, so a host-side
  GitHub credential was already assumed to exist.
- `moor import` already streamed a `git bundle` *into* a fresh sandbox
  over `docker exec -i … "cat > /tmp/import.bundle"` (the read-only
  rootfs defeats `docker cp` even onto a writable tmpfs mount). Every
  other direction is the mirror of that same stream.

## Decision

Every credentialed git operation moor performs now runs on the host,
authenticated with the host's own credential, and the result crosses
into or out of the sandbox only as a `git bundle` stream through `docker
exec`'s stdin/stdout — never a bind mount, never `docker cp`.

1. **The host's credential is `gh auth token`.** `host_github_token()`
   (`cli/src/commands/mod.rs`) resolves it; `gh` already authenticates the
   operator's account and `gh pr create` already assumed it. `None` means
   nothing to ship/clone/pull with — callers fail loudly rather than
   falling back to any in-sandbox value.
2. **`moor ship`** still commits inside the sandbox (so the commit keeps
   ADR-0007's synced identity), then streams just the shipped branch out
   as a bundle (`git bundle create - <branch>` via `docker exec`), fetches
   it into an ephemeral host-side scratch repo, and pushes from there with
   the host credential. A missing credential is checked before the branch
   is even created, so a failed push never leaves a half-shipped spec.
3. **`moor new --github`** clones the freshly created (private) repo on
   the host, bundles it, and streams the bundle into the sandbox's
   workspace, exactly as `moor import` already did for a local source
   repo.
4. **`moor pull`** fetches the sandbox's current branch from its GitHub
   remote into a host scratch repo, bundles `FETCH_HEAD`, streams it in,
   and merges with `--ff-only` inside the sandbox — no `ssh` needed in
   either direction (ADR-0010 is unaffected: the sandbox still has none).
5. **`GITHUB_TOKEN` is removed from the sandbox compose env entirely**
   (`compose.rs`'s `render_with` filters it out of the secret env lines
   even if an old manifest still lists it), and `Manifest::new` no longer
   seeds it among a new project's default secrets.
6. Every scratch repo is ephemeral: created under a per-invocation temp
   directory and removed before the command returns. The host keeps no
   standing clone.

`moor ship` stays the single entry point for publishing; its contract to
the operator is unchanged — only where the credential lives changes.

## What this deliberately does not do

- **Does not change commit authorship.** The commit a spec ships still
  happens inside the sandbox, under ADR-0007's synced git identity;
  only the push moves to the host.
- **Does not touch `moor import`'s existing host-side bundle for a local
  source repo**, or `gh pr create`'s already-host-side behaviour.
- **Does not widen the egress allowlist.** With pushes/clones/pulls now
  leaving from the host, the sandbox's egress allowlist no longer needs
  `github.com` for moor's own git — tightening it is a follow-up, not
  part of this change.
- **Does not mediate `moor shell`.** An operator who opens an interactive
  shell and runs `git push` by hand is outside this flow, as already
  noted in docs/THREAT-MODEL.md.

## Consequences

- No value derived from `GITHUB_TOKEN` is ever present in the sandbox's
  environment, `docker inspect`, or `docker exec ps` output — the token
  only ever exists as a short-lived env var set around a single host
  `git` invocation (`run_git_as_host`), named distinctly
  (`MOOR_HOST_GH_TOKEN`) from the sandbox secret it replaces.
- Shipping, creating with `--github`, and pulling all now depend on `gh
  auth login` having been run on the host; each fails with that exact
  instruction, before touching the sandbox, when it hasn't been.
- `moor run <p> -- git push`, run by an operator directly rather than
  through `moor ship`, will no longer authenticate — the sandbox carries
  no credential for it to use. `run_cmd.rs` still tags such a push
  distinctly in the audit chain even though it can no longer succeed
  against GitHub.
