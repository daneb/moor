# Recipes and logs

Taking a spec through every step by hand — write it, check it, approve it,
plan it, check the plan, approve it, build it, approve the merge — gets
old, even with `moor go`. `moor recipe` drives that whole sequence from
one loosely-described outcome, stopping at the same checkpoints for you:

```bash
moor recipe my-app docs/examples/recipe/greet-function.recipe.md
```

The recipe file is just YAML front matter (`slug`, `scope`) followed by
free text describing what you want — not a DSL, deliberately: the free
text goes to the agent close to verbatim, so writing it loosely is the
intended way to use this, not a limitation of it. The recipe's spec
becomes the active one, so `moor next` and `moor approve` follow it.

Every run stops and tells you exactly what to do whenever a decision is
actually yours:

```
PAUSED for your approval. Review it and decide:

    moor approve      (shows it first, then asks)
    moor reject "why"

...then re-run this recipe to continue.
```

Content-authoring checks (spec/plan) get a bounded, tool-restricted
self-correction loop — a failing check's own output is fed to an agent
that can only use `Write`/`Edit`, never Bash, so it can fix exactly what
the check named without going and implementing the feature instead. The
actual build gets no such auto-retry beyond a small attempt cap — that
step has full tool access, and iterating on it unattended is exactly the
scope creep this project's security posture argues against, so it stops
and hands the evidence to you instead. See
[ADR-0005](decisions/0005-recipe.md) for the full design, and
[docs/examples/recipe](examples/recipe) for a real run's output —
including two real bugs this exercise found, with fixes.

Since the build step can take a while and a recipe's own progress
narration used to only go to the terminal that launched it,
`moor logs <name> [--follow]` reads the same tamper-evident audit trail
and shows just the critical events — stage transitions, check attempts,
pauses for approval — from any terminal, live:

```bash
moor logs my-app --follow
```

```
[17:31:07] [logs-verify] stage: spec
[17:31:07] [logs-verify] gate attempt 1/4: fail
[17:31:14] [logs-verify] gate attempt 2/4: fail
[17:31:25] [logs-verify] gate attempt 3/4: pass
[17:31:25] [logs-verify] stage: spec_approval
[17:31:25] [logs-verify] PAUSED — waiting on: spec_approval
```

See [ADR-0006](decisions/0006-recipe-logs.md) — verified live,
watching a second terminal pick up real stage transitions and gate
failures within half a second of a real recipe run producing them.
