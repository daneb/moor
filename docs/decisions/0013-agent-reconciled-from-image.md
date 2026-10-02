# ADR-0013: A turn's agent is reconciled against the sandbox image, not read raw

**Status:** Accepted
**Date:** 2026-10-02

## Context

A project's manifest carries an `agent` field (SPEC-0010, the `--agent`
flag) that decides which coding-agent CLI a turn runs — `claude`,
`copilot`, or `kiro`. That field defaults to `Claude`:

```rust
/// The agent the sandbox is set up for; see `Agent`. Defaults to
/// Claude so manifests written before this field still load.
#[serde(default)]
pub agent: Agent,
```

The default exists for a good reason — manifests written before the field
was added still load. But it opens a gap that showed up in the field.

The agent is **orthogonal to the image in the type system but not in
reality**: Claude ships in every image, Copilot only in the opt-in
`moor/copilot*` layer, and a sandbox built from a `moor/copilot-python`
image runs no `claude` binary at all. `moor new`/`import` keep the two in
agreement — they set `agent` explicitly and compose a matching image via
`resolve_image`. But a manifest that is hand-edited, or was written before
the field existed, can end up with a **Copilot image and a defaulted-to-
Claude agent**.

When that happens, `run_turn` read the raw `m.agent` (Claude), ran the
`claude` CLI inside a Copilot sandbox, and the CLI — finding no Claude
credential, because none belongs there — printed its own interactive
advice:

```
the agent reported an error for this turn
Not logged in · Please run /login
```

`/login` is useless in the sandbox (no interactive session), and worse,
it points the operator at the wrong fix entirely: the sandbox was never
meant to run Claude. A real `sc-analyze` project exhibited exactly this —
`image: moor/copilot-python:latest` with no `agent:` line.

## Decision

**All turn and credential logic reads the agent through a single
reconciling accessor, `Manifest::agent()`, never the raw field.** When the
stored agent is the default `Claude` *and* the image is a Copilot image,
the effective agent is `Copilot`:

```rust
pub fn agent(&self) -> Agent {
    if self.agent == Agent::Claude && image_is_copilot(&self.image) {
        return Agent::Copilot;
    }
    self.agent
}
```

`image_is_copilot` is the one place that recognises the opt-in layers
(`moor/copilot`, `moor/copilot-<lang>`), reusing the logic `resolve_image`
already depended on rather than re-deriving it.

The read-sites that select the CLI, parse its output, build its argv,
refuse unrestrainable roles, write the audit entry, and emit the failure
hint all go through `m.agent()`. `run_turn` resolves it once into a local
binding so one reconciled value is used consistently across the turn.
`moor up`'s Copilot device-token discovery (ADR-0012) uses it too, so a
Copilot sandbox gets its device token even when the field is absent.

The **write-sites are untouched**: `new`/`import` still assign the
operator's explicit `--agent` choice to the raw field. Reconciliation is a
read-time correction for manifests that are already on disk, not a change
to how new ones are produced.

## Why reconcile at read, not migrate on load

The alternative was to rewrite the manifest on load — fill in the missing
`agent:` and save. Rejected:

- **Loading should not have a write side-effect.** `Manifest::load` is
  called on read-only paths and in contexts where writing back is
  surprising; a silent rewrite-on-read is a worse surprise than a derived
  value.
- **The raw field is still the operator's record of intent.** Preserving
  it means an explicit choice is never second-guessed — `agent()` only
  ever *upgrades* a defaulted Claude on a Copilot image; an explicitly set
  agent (including Claude, Kiro, or Copilot) passes through unchanged.
- **One boundary, not many.** Every path that matters already had to call
  something to get the agent; routing those through `agent()` closes the
  gap at the single read boundary without a migration step to get wrong.

## What this deliberately does not do

- **It does not validate at write time.** `new`/`import` already compose
  agent and image consistently via `resolve_image`; the gap was only in
  reading pre-existing or hand-edited manifests, which the accessor
  closes. A `selftest`/lint check that *surfaces* an on-disk
  agent/image disagreement (rather than silently reconciling it) is a
  reasonable follow-up, deliberately left out of this change's surface.
- **It does not reconcile in the other direction.** A Claude image with
  an explicit `agent: copilot` is left as the operator set it — that is a
  misconfiguration the operator asked for, not a missing-field default,
  and silently flipping it would hide a real mistake.
- **It does not touch non-macOS or non-Copilot paths.** Claude and Kiro
  images are not recognised as Copilot, so their stored agent passes
  through verbatim.

## Consequences

- A Copilot sandbox whose manifest omits `agent:` now runs the `copilot`
  CLI and, on a missing credential, shows Copilot's host-side fix
  (`copilot /login`) instead of Claude's misleading `/login` prompt.
- The fix is structural: there is one accessor and one `image_is_copilot`
  predicate, so a future agent layer that gains its own image prefix
  extends the predicate in one place.
- The raw `agent` field remains the serialized source of intent, so
  `moor` tooling and tests that assert on the stored value are unaffected;
  only the *effective* agent used to run a turn is reconciled.
- Verified by unit tests over the real argv builder and the manifest
  accessor (no mocking of the behaviour under test): a defaulted Copilot
  manifest reports `Copilot` from `agent()`, drives the `copilot` CLI, and
  yields the Copilot auth hint; an explicit agent and a non-Copilot image
  are left untouched.
