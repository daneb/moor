---
id: TASKS-0026
slug: ui-gate-uses-pipeline-verb
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 build the gate action through a pipeline-verb helper, with tests
- criteria: AC-1, AC-2, AC-3
- files: cli/src/ui/api.rs
- budget: 60
- exit: tests ui::api::tests::gate_argv_dispatches_through_pipeline and ui::api::tests::gate_label_names_moor_pipeline both pass

