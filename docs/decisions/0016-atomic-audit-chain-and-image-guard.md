# ADR-0016: Atomic, serialised audit-chain appends with a tolerant reader; and refusing cross-agent image composition

**Status:** Accepted
**Date:** 2026-11-09

## Context

Two defects surfaced from a real `sc-analyze` project on disk. Both live
in `cli/src`; neither was a one-off.

### 1. A non-atomic append could corrupt the audit chain

The audit chain (`keel.chain/1`) is a Merkle-style hash chain: each
`ChainEntry.hash` commits to the previous entry's hash, so any edit,
deletion, or reorder breaks every entry after it. `audit::append_entry`
is the sole writer. It computed the next entry from `last_entry(path)` —
which read the file and parsed the **last non-empty physical line** as one
JSON object — then re-opened the file in append mode and emitted the new
line with `writeln!`.

Two fragilities:

- **The reader assumed exactly one object per line and crashed otherwise.**
  `last_entry` called `serde_json::from_str` on the whole last line and
  propagated any error. Because `moor next`, `moor ui`, `moor studio`, and
  every `docker exec` logging path all read the chain to compute the next
  seq, a single malformed line stopped *every* command for that project
  with `trailing characters at line 1 column N`. One bad byte bricked the
  project.
- **The write was not guaranteed to be one line, nor serialised.**
  `writeln!` may issue more than one `write` syscall, and nothing
  serialised two `moor` processes appending to the same file (e.g. a `ui`
  exec-log firing while another command appended). The observed corruption
  was two whole records on one physical line with no separator and a
  duplicated `seq` — exactly what an interleaved or half-completed append
  produces.

### 2. `resolve_image` could compose an image that cannot exist

`manifest::resolve_image(language_image, agent)` prepends an agent prefix
to a `moor/` image unless it is already in *the chosen agent's* layer. It
did not check whether the image was already in a *different* agent's layer.
So `resolve_image("moor/kiro-python", Copilot)` returned
`moor/copilot-kiro-python` — never built, so `docker compose up` fell
through to a registry pull and failed with `pull access denied …
repository does not exist`. The browser console reached the same path:
`ui::data::update_manifest` runs `resolve_image(image, m.agent)` when an
operator edits the image, so setting agent and image to disagree wrote an
impossible image into the manifest.

## Decision

**Appends are atomic and serialised; the reader is tolerant; and
`resolve_image` refuses to stack a second agent prefix.**

- **One line, one write (AC-1).** `append_entry` builds the serialised
  entry plus its trailing newline into a single buffer and emits it with
  one `write_all`. With `O_APPEND`, each `write(2)` lands atomically at
  end-of-file, so a single-buffer write makes the whole entry one atomic
  append — no `writeln!` split.
- **Serialised across processes (AC-2).** The whole read-compute-write
  cycle holds an exclusive advisory lock (`rustix::fs::flock`) on the
  chain file, so two `moor` processes cannot each read the same tail,
  compute the same next seq, and interleave their writes. The lock is
  advisory and process-local to moor — nothing in a sandbox has the path —
  so it adds no new trust assumption, only mutual exclusion between moor's
  own writers. `rustix` was already resolved transitively (via
  `crossterm`), so declaring it direct added no new dependency fetch.
- **Tolerant reader (AC-3, AC-4).** `last_entry` no longer assumes one
  object per line. It decodes every object on the last physical line with
  a streaming deserializer and takes the last — recovering the true tail
  from a glued line — and if a line yields no entry at all it walks back to
  the previous non-empty line rather than erroring. A single corrupt
  historical line now degrades to "read the last good entry", never a hard
  crash of unrelated commands. This mirrors the truncation case
  `verify_chain` already treats as non-tampering.
- **No cross-agent composition (AC-5, AC-6).** `resolve_image` consults a
  single list of agent prefixes (`copilot`, `kiro`); if the image is
  already in *another* agent's layer it returns the image unchanged
  instead of prefixing it again. A property-style test proves no input
  across every agent and every apex/language variant yields a
  double-agent prefix.
- **Rejected at the edit (AC-7).** `ui::data::update_manifest` calls a
  pure `cross_agent_image_error(image, agent)` helper and fails the edit
  with a message naming the mismatch, rather than saving an image that
  cannot be pulled or built.

## Why these choices

- **Lock the cycle, not just the write.** The corruption was a race on the
  *seq computation*, not only the write. Locking from before `last_entry`
  to after the write makes next-seq-and-append one critical section;
  locking only the write would still let two readers pick the same seq.
- **Tolerate on read, do not rewrite on read.** As with ADR-0013, reads
  have no write side-effect. The reader recovers the last good entry; it
  never rewrites the file to "repair" it. Repairing a damaged file on disk
  is deliberately out of scope (it was done by hand for `sc-analyze`).
- **Refuse, do not silently rename.** `resolve_image` leaving a
  cross-agent image unchanged, and `update_manifest` refusing it, both
  surface the operator's mistake rather than inventing an image. This
  extends ADR-0013, which made the image the source of truth for the
  agent: a new impossible image is never *formed*.
- **The hash framing is untouched.** `compute_hash` and the `keel.chain/1`
  framing are byte-for-byte unchanged; the golden-entry and tamper tests
  stay green.

## What this deliberately does not do

- **It does not repair already-corrupted chain files.** A `moor`/`keel`
  repair subcommand for existing damage is a separate change.
- **It does not change the hash framing or the `keel.chain/1` schema.**
  The format is keel's.
- **It does not migrate manifests already holding an impossible image.**
  `Manifest::agent()` already reconciles a missing/stale agent field
  against the image (ADR-0013); this change stops a *new* impossible image
  being formed.

## Consequences

- A project whose chain has one damaged line keeps working: `moor next`,
  `moor ui`, and logging read past it to the last good entry.
- Concurrent moor writers to one chain can no longer interleave; the file
  keeps its one-object-per-line invariant under contention, proven by a
  multi-threaded append test that then verifies the chain.
- Editing a project's image to a different agent's layer fails at the edit
  with a clear message, instead of surfacing much later as a registry-pull
  failure at `up` time.
- Verified by unit tests over real files and real `resolve_image` inputs,
  exercising the behaviour under test directly rather than through a
  substitute double.
