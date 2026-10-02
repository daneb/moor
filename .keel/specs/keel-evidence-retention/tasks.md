---
id: TASKS-0015
slug: keel-evidence-retention
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 Raw run evidence is not tracked
- criteria: AC-1
- files: scope
- budget: 28
- exit: `git check-ignore -q .keel/runs` exits 0

### T-2 Verified bundles are tracked
- criteria: AC-2
- files: scope
- budget: 28
- exit: `! git check-ignore -q .keel/bundles` exits 0

### T-3 The evidence chain is not tracked
- criteria: AC-3
- files: scope
- budget: 28
- exit: `git check-ignore -q .keel/chain.jsonl` exits 0

### T-4 A bundle verifies on its own
- criteria: AC-4
- files: scope
- budget: 28
- exit: `keel export --verify "$(ls -1t .keel/bundles/*.tar.gz | head -1)"` exits 0

### T-5 One command produces the bundle to commit
- criteria: AC-5
- files: scope
- budget: 28
- exit: `grep -qE '^evidence:' Makefile` exits 0

### T-6 The retention model is documented
- criteria: AC-6
- files: scope
- budget: 28
- exit: `grep -rqi 'bundle' docs/EVIDENCE.md` exits 0

### T-7 Existing CLI tests stay green
- criteria: AC-7
- files: scope
- budget: 28
- exit: `cargo test --manifest-path cli/Cargo.toml` exits 0

