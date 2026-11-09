---
id: SPEC-0020
slug: audit-chain-atomicity-and-agent-image
schema: keel.spec/1
status: approved
scope:
- cli/src/audit.rs
- cli/Cargo.toml
- cli/src/manifest.rs
- cli/src/ui/data.rs
- docs/decisions/**
budget:
  criteria: 8
  lines: 520
verified_at: 2026-11-09
---

# Atomic audit-chain appends and a tolerant reader; reject cross-agent image composition

## Context

Two latent defects surfaced from a real `~/.moor` project (`sc-analyze`).
Both are in `cli/src`; neither is a one-off data glitch.

**1. The audit chain can be corrupted by a non-atomic append.** The sole
writer of a project's hash chain is `audit::append_entry`
(`cli/src/audit.rs`): it computes the next entry from
`last_entry(path)` — which reads the file and parses the *last non-empty
line* — then re-opens the file in append mode and emits the line with
`writeln!`. Two things make this fragile:

- `last_entry` assumes one JSON object per physical line. It calls
  `serde_json::from_str` on that whole last line and propagates any parse
  error. So the moment a line holds anything but exactly one object, the
  read side fails with `trailing characters at line 1 column N`, and —
  because `moor next`, `moor ui`, `moor studio` and every `docker exec`
  logging path all go through the same chain read — *every* command for
  that project stops working. One malformed byte bricks the project.
- The write side is not guaranteed to land as one line. `writeln!` on a
  `File` can issue more than one `write` syscall, and nothing serialises
  two `moor` processes (e.g. a `ui` exec-log firing while another command
  appends) against the same file. The observed corruption was two whole
  records on one physical line with no separator and a duplicated `seq`,
  which is exactly what an interleaved or half-completed append produces.

The chain is a Merkle-style hash chain (`ChainEntry.hash` commits to
`prev_hash`), so a corrupt or interleaved append also threatens the one
property the file exists to provide. The fix is twofold: make an append
atomic and serialised, and make the reader tolerant enough that a single
malformed historical line degrades to a recoverable error, never a hard
crash of unrelated commands.

**2. `resolve_image` can compose an image that cannot exist.**
`manifest::resolve_image(language_image, agent)` prepends an agent prefix
to a `moor/` image unless that image is already in *the chosen agent's*
layer. It does not check whether the image is already in a *different*
agent's layer. So `resolve_image("moor/kiro-python", Copilot)` returns
`moor/copilot-kiro-python` — a double-prefixed name that is never built,
so `docker compose up` falls through to a registry pull and fails with
`pull access denied ... repository does not exist`. The UI reaches this
same path: `ui::data::update_manifest` runs
`resolve_image(image, m.agent)` when an operator edits the image, so
setting the agent and image to disagree writes an impossible image into
the manifest. A composition that would stack a second agent prefix onto an
image already in another agent's layer is never valid and must be refused
at the point it is formed, not discovered at `up` time.

Out of this spec: repairing already-corrupted chain files on disk (done
by hand for `sc-analyze`), and any change to the hash framing or
`keel.chain/1` schema — the format is keel's and stays byte-for-byte as
the golden-entry test pins it.

## Acceptance criteria

### AC-1 An append writes the whole entry as a single line in one write

WHEN `audit::append_entry` writes a chain entry THE SYSTEM SHALL emit the
serialised entry and its terminating newline to the file in a single
`write_all` of one buffer, rather than through multiple writes.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact audit::tests::append_writes_entry_as_one_line 2>&1 | grep -q '1 passed'` exit 0

### AC-2 Concurrent appends never interleave on one line

WHEN two appends to the same chain file race THE SYSTEM SHALL serialise
them so that every physical line in the resulting file parses as exactly
one JSON object and no line holds two concatenated records.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact audit::tests::concurrent_appends_do_not_interleave 2>&1 | grep -q '1 passed'` exit 0

### AC-3 The reader tolerates a last line that holds more than one record

WHEN `last_entry` reads a chain whose final non-empty line contains more
than one concatenated JSON object THE SYSTEM SHALL return the last
well-formed entry on that line rather than propagating a parse error.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact audit::tests::last_entry_reads_final_record_of_a_glued_line 2>&1 | grep -q '1 passed'` exit 0

### AC-4 A malformed historical line does not crash an otherwise valid chain read

IF a chain file contains one unparseable line WHEN `last_entry` is called
THEN THE SYSTEM SHALL recover the latest valid entry and SHALL NOT return
an error that would stop `moor next` for that project.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact audit::tests::malformed_line_does_not_crash_read 2>&1 | grep -q '1 passed'` exit 0

### AC-5 resolve_image refuses to stack a prefix onto another agent's layer

WHEN `resolve_image` is given a `moor/` image already in a different
agent's layer than the requested agent THE SYSTEM SHALL return the image
unchanged rather than prepending a second agent prefix.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact manifest::tests::resolve_image_does_not_cross_compose_agents 2>&1 | grep -q '1 passed'` exit 0

### AC-6 No code path can produce a double-agent image name

WHEN `resolve_image` is exercised across every agent and the apex and
language variants of every agent layer THE SYSTEM SHALL never return a
repository whose name begins with one agent prefix immediately followed by
another agent prefix.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact manifest::tests::resolve_image_never_double_prefixes 2>&1 | grep -q '1 passed'` exit 0

### AC-7 Editing a manifest to a mismatched agent and image is rejected at the edit

WHEN `ui::data::update_manifest` is asked to set an image that is in a
different agent's layer than the manifest's agent THE SYSTEM SHALL fail
the edit with a message naming the mismatch, rather than saving an image
that cannot be pulled or built.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact ui::data::tests::update_manifest_rejects_cross_agent_image 2>&1 | grep -q '1 passed'` exit 0

### AC-8 The behaviour change is recorded by an ADR

WHEN this change lands THE SYSTEM SHALL add an ADR describing atomic
serialised chain appends, the tolerant reader, and the cross-agent
image-composition refusal. (That both suites stay green is enforced by
G2's own build/test checks, so it is not duplicated as an oracle here —
an "always green" check cannot be sensitive at the base.)

oracle: cmd `ls docs/decisions/*audit-chain* docs/decisions/*atomic* 2>/dev/null | head -1 | grep -q .` exit 0

## Out of scope

- **Repairing corrupted chain files on disk.** The `sc-analyze` chain was
  split back into one-object-per-line by hand; building a `moor`/`keel`
  repair subcommand for existing damage is a separate change.
- **The hash framing or `keel.chain/1` schema.** The format is keel's and
  stays byte-for-byte identical; the golden-entry test must remain green.
- **Changing authorship, writer identity, or who holds the pen.** moor
  remains the sole host-side writer of the chain.
- **Reconciling an already-saved impossible image.** `Manifest::agent`
  already reconciles a missing/stale agent field against the image; this
  spec stops a *new* impossible image being formed, it does not add
  migration for manifests already on disk.
