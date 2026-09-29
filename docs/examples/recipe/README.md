# Example: a feature end to end from a loosely-described recipe

`greet-function.recipe.md` in this directory is the exact recipe used to
verify [`moor recipe`](../../decisions/0005-recipe.md) end to end, on a
real project, against real checks. Nothing below is hypothetical.

> The output below is as recorded at the time, so it shows the pipeline's
> underlying `keel` commands and moor's older messages. A paused recipe
> now points you at `moor approve` / `moor reject` instead; see
> [ARCHITECTURE.md](../../ARCHITECTURE.md#what-moor-is-built-from) for how
> they map.

```bash
moor recipe my-app docs/examples/recipe/greet-function.recipe.md
```

## What actually happened, run for real

**First invocation** — scaffolds the spec, authors it via a
`Write`-only agent call, gates it, and stops at the first human
checkpoint:

```
==> recipe 'greet-function' for project 'recipe-test'
==> scaffolded .keel/specs/greet-function/ — authoring the spec now
Spec written to `.keel/specs/greet-function/spec.md` — 5 acceptance
criteria ... satisfying G0 (id, slug, schema, status: draft, scope, budget).

==> [greet-function] stage: spec  next: keel gate g0 greet-function
G0 pass — 16 passed, 0 failed, 0 blocked

==> [greet-function] stage: spec_approval  next: keel approve greet-function --stage spec

PAUSED for human approval. Review the change, then run:

    moor run recipe-test -- keel approve greet-function --stage spec

...and re-run this recipe to continue.
```

G0 passed on the agent's first attempt here — it doesn't always. After
approving and re-running, `keel plan` scaffolded `plan.md`/`tasks.md`,
and G1 **did** fail, twice, on real problems:

```
FAIL     task-files-in-scope
         actual:   no files on: T-1 (line 16), T-2 ..., T-5 (line 40)
FAIL     rollback-stated
         actual:   both are empty — "git revert" is a legitimate answer, silence is not

==> gate failed (attempt 1/4) — asking the agent to fix exactly that
Fixed both failures: filled in `files: src/greet.js` on all five tasks
in `tasks.md`, and set `rollback: 'git revert'` ...

FAIL     wave-isolation
         actual:   wave 1: T-1 and T-2 both claim src/greet.js, ... — add a `depends_on` to order them

==> gate failed (attempt 2/4) — asking the agent to fix exactly that
Added `depends_on` chains (T-2→T-1, T-3→T-2, T-4→T-3, T-5→T-4) ...

G1 pass — 17 passed, 0 failed, 0 blocked

==> [greet-function] stage: plan_approval  next: keel approve greet-function --stage plan
```

Fixing the first two failures introduced a *third*, different failure
(`wave-isolation`) — the agent's own first fix created a new problem,
which the next retry caught and fixed. This is the bounded self-correction
loop working as designed, not edited for the writeup.

After approving the plan, `keel run` drove the actual build (`src/greet.js`,
via keel's own `claude-code` driver — full tool access, unrestricted, since
this step is the real implementation, not text authoring). This project's
`.keel/keel.toml` had no `lint` command configured and the agent didn't add
a test file alongside the code change, so G2/G2.5 stayed `BLOCKED` rather
than `pass` — `keel next` kept saying `stage: run`, and after
`max_run_attempts` (default 2) the recipe stopped rather than retrying
forever:

```
error: `keel run` still hasn't reached a human checkpoint after 2
attempt(s) — see .keel/runs/ for evidence; drive it by hand, then
re-run this recipe
```

This is the safety cap working, not a bug: an unconfigured or genuinely
stuck project would otherwise loop on `keel run` indefinitely, since a
`BLOCKED` gate is exactly as unresolved on attempt 50 as attempt 1.

## Two real bugs this exercise found in `moor recipe` itself

1. **`--allowedTools <tools...>` is variadic** — it swallows every bare
   argument after it until the next flag, including the prompt itself if
   the prompt comes second. `claude --print --allowedTools Write
   "<prompt>"` fails ("Input must be provided either through stdin or as
   a prompt argument"); `claude --print "<prompt>" --allowedTools Write`
   works. Fixed by ordering the prompt before the flag everywhere
   `moor recipe` calls `claude`.
2. **`keel`'s hard refusals (e.g. "already exists") print to stderr;**
   **gate results print to stdout.** `cli/src/proc.rs`'s existing
   `run_capture` only captured stdout, so every resume after the first
   run (`keel spec new` on an already-scaffolded spec) looked like empty
   output — which `moor recipe` misread as a real failure instead of the
   expected idempotent no-op. Fixed by adding `run_capture_combined`
   rather than changing `run_capture`'s behavior for its other callers.

See [ADR-0005](../../decisions/0005-recipe.md) for the full design and a
third finding (a concrete goal description makes Claude Code implement
the thing instead of writing a spec about it, regardless of prompt
wording — fixed with `--allowedTools`, not more careful phrasing).
