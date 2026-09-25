---
id: TASKS-0007
slug: moor-papercuts
schema: keel.tasks/1
---

# Tasks

### T-1 Host identity rendered as GIT_CONFIG env
- criteria: AC-1, AC-2
- files: cli/src/compose.rs, cli/templates/project.compose.yml.tmpl
- budget: 90
- exit: cmd `cargo test --manifest-path cli/Cargo.toml compose::tests::host_git_identity_is_rendered_as_git_config_env -- --exact 2>&1 | grep -q '1 passed'` exit 0

### T-2 keel pinned in the image
- criteria: AC-4
- files: images/base/Dockerfile, images/build.sh, docs/IMAGES.md
- budget: 30
- exit: cmd `grep -Eq '^ARG KEEL_VERSION=[0-9]+\.[0-9]+\.[0-9]+$' images/base/Dockerfile` exit 0

### T-3 e2e checks the approver in a new project
- criteria: AC-3
- files: tests/e2e.sh
- budget: 30
- depends_on: T-1, T-2
- exit: cmd `bash tests/e2e.sh` exit 0
