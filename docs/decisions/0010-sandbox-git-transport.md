# ADR-0010: sandbox git transport is HTTPS-with-injected-token, not SSH

**Status:** Accepted
**Date:** 2026-10-01

## Context

An imported project's sandbox could not refresh its branch from GitHub:

```
$ moor run altron -- git pull
error: cannot run ssh: No such file or directory
fatal: unable to fork
```

The first instinct — "egress is blocking GitHub" — was **wrong**, and
checking it directly is what found the real cause. From inside the
sandbox:

```
$ moor run altron -- sh -lc 'command -v ssh || echo NO ssh'
NO ssh
$ moor run altron -- curl -s -o /dev/null -w '%{http_code}\n' https://github.com
200
$ moor run altron -- git ls-remote https://github.com/<owner>/<repo>.git HEAD
<a real sha>    HEAD
```

So: **github.com is reachable over HTTPS** (it is on the proxy's base
allow-list, `proxy/allowlist.base.txt`), and **git over HTTPS works**.
The only thing broken is SSH — and by design:

- The base image installs no `ssh` client (`images/base/Dockerfile`
  deliberately carries no SSH, no sudo, no docker CLI — see
  [docs/IMAGES.md](../IMAGES.md) and [THREAT-MODEL.md](../THREAT-MODEL.md)).
- `moor import` preserves the source repo's `origin` as-is, which for a
  repo cloned on the host over SSH is an SSH URL — often with a host
  alias from the operator's `~/.ssh/config`, e.g.
  `git@github.com-sbg:Owner/repo.git`.

`git pull` against that SSH `origin` shells out to `ssh`, which isn't
there. The sandbox already standardised on HTTPS everywhere it talks to
GitHub itself: `moor new --github` clones over HTTPS, and `moor ship`
pushes over HTTPS, both with a token read from `$GITHUB_TOKEN` *inside*
the container by a git credential helper so it never lands in a host-side
argv. Pull was simply the one GitHub interaction that had no moor command
and so fell through to a raw `git pull` over the SSH `origin`.

## Decision

The sandbox's git transport to GitHub is **HTTPS with a token injected at
runtime**, never SSH. A new `moor pull` command
(`cli/src/commands/pull_cmd.rs`) makes refreshing a branch a first-class
operation that follows that rule:

1. **Derive an HTTPS URL from `origin`, whatever form it takes.**
   `https_github_url()` rewrites scp-style SSH (`git@github.com:o/r.git`),
   SSH-with-host-alias (`git@github.com-sbg:o/r.git`), `ssh://` URLs, and
   already-HTTPS URLs to `https://github.com/<owner>/<repo>.git`. It
   accepts only hosts whose label chain includes `github.com`, and
   validates `owner`/`repo` shape — so it never silently rewrites some
   other host. Eight unit tests cover the forms and the refusals.

2. **Pull over that HTTPS URL explicitly, with the token in a credential
   helper.** The pull runs
   `git -c credential.helper='!f() { echo username=x-access-token; echo
   "password=$GITHUB_TOKEN"; }; f' pull --ff-only <https-url> <branch>`.
   `$GITHUB_TOKEN` is expanded by the shell *inside the container* (the
   compose template injects it from the host's env/Keychain, ADR-0002's
   mechanism), so the token is never in this process's argv and so never
   in `docker inspect`/`ps` output on the host — the same pattern
   `moor new`/`moor ship` already use.

3. **Persist nothing.** Because the URL is passed explicitly rather than
   stored as a remote, no credential is ever written to `.git/config`.
   `origin` is left exactly as it was (SSH, alias and all).

`--ff-only`, because a sandbox refresh should fast-forward to the remote,
not create merge commits unattended.

Verified live on the `altron` project, whose `origin` is
`git@github.com-sbg:Dane-Balia_sbg/filenet-retriever.git`: `moor pull`
derived the HTTPS URL, pulled `master`, left `origin` unchanged, and left
`git config` with no persisted credential (`git config --list` showed no
token and no `https://...@` remote).

## What this deliberately does not do

- **Does not install an SSH client in the sandbox.** That would reopen a
  surface the image closed on purpose, to make one interaction work that
  HTTPS already handles. The sandbox reaches GitHub over HTTPS or not at
  all.
- **Does not rewrite `origin` to HTTPS.** The operator's `origin` (and
  their SSH host alias) is left intact; `moor pull` constructs the HTTPS
  URL per-invocation. This keeps the operator's own host-side git
  workflow (which does have SSH) unaffected.
- **Does not persist a token anywhere.** No `credential.helper store`, no
  token in a remote URL on disk. Each pull injects it fresh and discards
  it, matching `moor ship`.
- **Does not add egress.** `github.com` is already on the base
  allow-list; this ADR relies on that, it doesn't widen it.

## Consequences

- Refreshing a sandbox branch is now `moor pull` instead of a
  hand-assembled one-shot `git pull https://x-access-token:...@github.com/...`
  that an operator had to remember and that risked pasting a token into a
  shell history.
- A project that uses `moor pull` must have `GITHUB_TOKEN` resolvable
  (shell env or Keychain) for a private repo — the command says so and
  points at `moor secrets set <p> GITHUB_TOKEN` when the pull fails on
  auth. (`COPILOT_GITHUB_TOKEN`, used by the Copilot agent, is
  deliberately not reused here: pull auth and agent auth are separate
  concerns and may be separate tokens.)
- `moor pull` only knows github.com. A non-GitHub remote returns a clear
  "only knows how to pull from a github.com remote over HTTPS" error
  rather than guessing — consistent with the rest of moor being
  GitHub-shaped today (`moor new --github`, `moor ship`'s `gh pr create`).
- The broader rule is now explicit for any future git interaction the
  sandbox grows: do it over HTTPS with an in-container injected token,
  the way `new`, `ship` and now `pull` all do — not SSH.
