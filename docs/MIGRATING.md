# Migrating an existing project into moor

`moor new` is for starting something fresh. `moor import` is for
bringing something that already exists on your Mac — a project you've
been working on outside moor — into a sandbox without losing anything
and without ever bind-mounting the source directory.

```bash
moor import my-service --from ~/Repos/my-service
```

## What actually happens

1. **The image is auto-detected** from the source repo's root —
   `Cargo.toml` → `moor/rust`, `package.json` → `moor/node`,
   `pyproject.toml`/`requirements.txt` → `moor/python`, otherwise
   `moor/base`. Override with `--image` if the guess is wrong.
2. **A sandbox + egress pair starts**, same as `moor new`.
3. **History transfers via a `git bundle`, not a bind mount.** `git -C
   <from> bundle create ... --all` captures every branch and tag into one
   file. That file is streamed into the container (through `docker exec
   -i`'s stdin — not `docker cp`, which doesn't work against a read-only
   root filesystem even when the destination is a writable tmpfs mount;
   see the comment in `cli/src/commands/import_cmd.rs` if you're curious
   why), cloned from inside the container, then deleted on both sides.
   This is the one piece of host content that ever crosses into the
   container for an import — a deliberate, one-shot copy, not a standing
   mount. Every branch and tag the bundle carried is then materialized as
   a real local branch inside the sandbox, not left as a remote-tracking
   ref that a later step could silently drop.
4. **An existing GitHub remote is preserved.** If the source repo already
   has an `origin` pointing at `github.com`, that URL is set as the
   sandbox clone's `origin` too (replacing the bundle's own path, which
   no longer exists once step 3 finishes). This is what makes importing
   an existing GitHub project work cleanly — `git push` inside the
   sandbox goes to the same real repo it always did.
5. **No remote, and `--github` wasn't passed:** `origin` is left unset,
   with a note showing you the `git remote add` command to run once
   you've decided where this project should live.
6. **No remote, `--github` was passed:** a private GitHub repo is created
   (same confirm-then-create flow as `moor new --github`) and set as
   `origin`.
7. **An existing spec setup is respected.** If the repo already has one
   (a `.keel/` directory — see
   [ARCHITECTURE.md](ARCHITECTURE.md#what-moor-is-built-from)), moor
   leaves it alone and shows its current status, so nothing is
   overwritten. Otherwise the workspace is set up for specs, same as
   `moor new`.

Once it's in, run `moor use <name>` and then `moor next` to see where
things stand.

## Verified against a real project

This was tested against a real multi-branch Rust repo, not a toy
fixture. The output below is as recorded at the time; the setup messages
have since been reworded.

```
$ moor import keel-import-test --from ~/Repos/keel
==> importing /Users/.../Repos/keel as 'keel-import-test' (image: moor/rust:latest)
...
==> bundling /Users/.../Repos/keel (all branches and tags)
==> streaming the bundle into the sandbox and cloning it
Cloning into '.'...
   origin set to git@github.com:daneb/keel.git
==> checking for existing keel configuration
   .keel/keel.toml already present — leaving it as-is (not re-running `keel init`)
```

`git log` inside the sandbox matched the host exactly, both branches
(`master` and a `claude/...` working branch) came across, `cargo
--version` worked immediately (the `moor/rust` image auto-selected
correctly), and `moor selftest`/`moor audit --verify` both
passed. See `tests/e2e.sh` for the automated version (using a throwaway
local repo, since that scenario doesn't need real GitHub access).

Two real bugs surfaced building this, both fixed before it shipped:

- `docker cp` silently fails against the sandbox's read-only root
  filesystem, even when the destination is a writable tmpfs mount — fixed
  by streaming the bundle through `docker exec -i`'s stdin instead.
- Cleaning up a bundle-path `origin` remote (when there's no real remote
  to replace it with) also deletes that remote's tracking refs — which
  would have silently dropped every branch but the default one. Fixed by
  materializing every branch as a local one before touching `origin` at
  all.
