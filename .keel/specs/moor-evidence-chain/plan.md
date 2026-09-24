---
id: PLAN-0005
slug: moor-evidence-chain
schema: keel.plan/1
blast:
  depth: 2
  declared:
  - cli/src/audit.rs
  - cli/src/posture.rs
  - cli/src/paths.rs
  - cli/src/main.rs
  - cli/src/commands/mod.rs
  - cli/src/commands/run_cmd.rs
  - cli/src/commands/audit_cmd.rs
  - cli/src/compose.rs
  - cli/templates/project.compose.yml.tmpl
  - tests/e2e.sh
  computed:
  - cli/src/paths.rs
  - cli/src/commands/mod.rs
  - cli/src/main.rs
  - cli/src/commands/run_cmd.rs
  - cli/src/audit.rs
  - cli/src/compose.rs
  - cli/src/commands/audit_cmd.rs
  - cli/src/commands/use_cmd.rs
  - cli/src/commands/keel_cmd.rs
  - cli/src/commands/logs.rs
  - cli/src/commands/up.rs
  computed_lines: 1575
rollback: git revert the merge; chains written in keel.chain/1 stay verifiable by keel, and each sealed legacy chain is untouched in chain.legacy.jsonl
verified_at: 2026-09-24
---

# Design — moor-evidence-chain

## Approach

The seam is `audit::append_chained`, the one function every chain writer
already goes through (`log_exec`, `fold_egress_log`, recipe events,
`agent-turn`). Change the format there and every writer moves with it.

**Format (AC-1).** `ChainEntry` gains `schema` (`keel.chain/1`) and `writer`
(`moor`). Its hash becomes keel's: SHA-256 over length-framed
`(name, bytes)` pairs, in this order: `prev_hash`, `seq` as a decimal string,
`ts`, `kind`, `writer`, and `data` serialised by `serde_json`. Seq starts at 1.
A golden entry written by keel's own `chain::append` is embedded in the tests.
Moor must reproduce its hash and verify it, which proves the two
implementations agree byte for byte.

**Legacy seal (AC-2).** Before appending, if the chain file exists and its
first line has no `schema`, rename it to `chain.legacy.jsonl`. Verify it with
the old hash function, kept as `legacy_hash`. Then start the new chain with
`legacy_seal { legacy_file, legacy_head, legacy_entries, legacy_verified }`.
If a legacy file already exists, refuse rather than overwrite it.
`moor audit --verify` checks both files.

**Sink (AC-6).** `fold_sink_lines(chain, lines)` is pure over text, so it can
be tested without Docker. Each line is redacted with the manifest's secrets and
parsed as `{kind, data}`, then appended under its own `kind` with
`source: "sandbox"`. A line that doesn't parse is recorded as
`sink_malformed { line_no, line }`. `fold_sink` pulls
`/run/moor-sink/keel.jsonl` with `docker exec cat` and tracks a
`.sink-offset`, using the same stale-offset reset as the egress fold.
`log_exec` calls it first, so keel's payloads land before the exec that
produced them. That covers `moor run`, `moor keel` and `recipe` with no edits
to `recipe.rs`. `moor audit` folds it too.

**Posture (AC-3, AC-4).** New `posture.rs`. `derive(container, networks)` is
pure over `docker inspect` JSON:

| property | proven when | violated when |
| --- | --- | --- |
| `fs.read_only_root` | `ReadonlyRootfs` true | false |
| `mounts.no_host_bind` | `Mounts` present, none of type `bind` | any bind |
| `user.non_root` | `Config.User` set and not root/0 | empty, `root`, `0` |
| `caps.dropped_all` | `CapDrop` holds `ALL` | array without `ALL` |
| `privileges.no_new` | `SecurityOpt` has `no-new-privileges` | array without it |
| `privileged.off` | `Privileged` false | true |
| `network.internal_only` | every attached network `Internal` | any not |

A field that's missing, null or the wrong type gives `unproven`. `attest(name,
m)` runs `docker inspect` on the container and its networks, writes the host
copy at `~/.moor/projects/<name>/posture.json`, and pipes it with
`run_with_stdin_file` into `docker exec -i -u root <sandbox> sh -c 'cat >
/run/moor/posture.json && chmod 0444 /run/moor/posture.json'`. It then appends
`attest { sha256, runtime: "moor", image_digest, properties }`. `compose_up`
calls it after `sync_git_identity`. A failure there is a warning, not an error:
keel already blocks a run whose required posture has no attestation.

**Compose (AC-5).** The sandbox gains `tmpfs` entries
`/run/moor:mode=0755` (root-owned, so the `agent` user can read but not write)
and `/run/moor-sink:mode=1777`, plus `KEEL_RUNTIME_ATTESTATION` and
`KEEL_CHAIN_SINK`. There are still no bind mounts, and the existing test that
guards this stays.

**Push (AC-7).** `run_cmd` renames kind `git-push` to `push`.
`push_target(argv)` is pure: it returns the `-C` dir (default `/workspace`) and
the ref (the refspec's source side, else `HEAD`). Before the push, Moor runs
`git rev-parse --verify <ref>^{commit}` in the container, then logs
`push { argv, exit_code, ref, commit }` through a `log_exec` variant that takes
extra data. The commit is null if the lookup fails.

**End to end (AC-8).** `e2e.sh` greps for `"kind":"push"`, runs
`moor keel spec new` inside the sandbox so that G0 emits a gate payload, and
checks the host chain for `attest`, a sandbox-sourced `gate`, and `egress`. It
then copies the chain into a scratch `.keel/` and runs `keel chain verify`,
using `$KEEL` or else `keel`. This needs a keel release containing keel#7 and
keel#8, built into the image.

## Blast radius

<!-- generated by `keel plan`; edits here are overwritten -->

Computed from the import graph at depth 2: **11 files, 1575 lines**.

| depth | file | lines |
| --- | --- | --- |
| scope | `cli/src/paths.rs` | 90 |
| scope | `cli/src/commands/mod.rs` | 279 |
| scope | `cli/src/main.rs` | 212 |
| scope | `cli/src/commands/run_cmd.rs` | 81 |
| scope | `cli/src/audit.rs` | 458 |
| scope | `cli/src/compose.rs` | 131 |
| scope | `cli/src/commands/audit_cmd.rs` | 105 |
| +1 | `cli/src/commands/use_cmd.rs` | 57 |
| +1 | `cli/src/commands/keel_cmd.rs` | 56 |
| +1 | `cli/src/commands/logs.rs` | 95 |
| +1 | `cli/src/commands/up.rs` | 11 |

_Scope globs matching no indexed file (new files, or a typo): cli/src/posture.rs, cli/templates/project.compose.yml.tmpl, tests/e2e.sh_


## Rollback

`git revert` — replace this if the change needs a different rollback.
