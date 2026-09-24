pub mod ask_cmd;
pub mod audit_cmd;
pub mod down;
pub mod import_cmd;
pub mod keel_cmd;
pub mod logs;
pub mod new_cmd;
pub mod recipe;
pub mod run_cmd;
pub mod secrets_status;
pub mod selftest;
pub mod shell;
pub mod status;
pub mod studio_cmd;
pub mod up;
pub mod use_cmd;
pub mod view;

use crate::{compose, manifest::Manifest, paths};
use anyhow::{Context, Result};
use std::io::{self, Write};

/// Of all known projects, the ones whose sandbox container is currently
/// running (`moor up`'d). A single `docker ps` covers every project at
/// once rather than shelling out per project.
pub(crate) fn running_projects(names: &[String]) -> Result<Vec<String>> {
    let (_status, out) = crate::proc::run_capture("docker", &["ps", "--format", "{{.Names}}"])?;
    let running: std::collections::HashSet<&str> = out.lines().collect();
    let mut matches = vec![];
    for name in names {
        let Ok(manifest_path) = paths::manifest_path(name) else {
            continue;
        };
        if !manifest_path.exists() {
            continue;
        }
        if let Ok(m) = Manifest::load(&manifest_path) {
            if running.contains(m.sandbox_container().as_str()) {
                matches.push(name.clone());
            }
        }
    }
    Ok(matches)
}

/// How a project name was arrived at, when the operator didn't spell it
/// out as a plain command-line argument — shown alongside the resolved
/// name so a command can never silently act on the wrong container.
#[derive(Debug)]
pub enum ProjectSource {
    Sticky,
    RunningUnique,
    SoleProject,
}

impl ProjectSource {
    fn describe(&self) -> &'static str {
        match self {
            ProjectSource::Sticky => "set via `moor use`",
            ProjectSource::RunningUnique => "only sandbox currently up",
            ProjectSource::SoleProject => "only project you have",
        }
    }
}

/// The auto-detection half of `resolve_project`'s precedence rule,
/// pulled out as a pure function of already-gathered inputs (no disk or
/// docker access) so "the one sandbox that's up wins, else the one
/// project you have, else ask" can be unit tested directly. Only reached
/// once an explicit `--project` and a `moor use` default have both come
/// up empty.
fn pick_running_or_sole_project(
    all: &[String],
    running: &[String],
) -> Result<(String, ProjectSource)> {
    if running.len() == 1 {
        return Ok((running[0].clone(), ProjectSource::RunningUnique));
    }
    match all.len() {
        0 => anyhow::bail!("no projects yet — try `moor new <name>`"),
        1 => Ok((all[0].clone(), ProjectSource::SoleProject)),
        _ if running.len() > 1 => anyhow::bail!(
            "more than one project is up ({}) — pass `--project <name>` or run `moor use <name>`",
            running.join(", ")
        ),
        _ => anyhow::bail!(
            "more than one project exists ({}) and none is up — run `moor up <name>`, `moor use <name>`, or pass `--project <name>`",
            all.join(", ")
        ),
    }
}

/// Resolve the project a command should act on when the operator didn't
/// name one explicitly: `moor keel`/`moor view` are meant to be used
/// without repeating the project on every call. Precedence: an explicit
/// `--project` flag; the sticky default set by `moor use`; the one
/// project whose sandbox is actually running right now (keel ships
/// natively in every sandbox, so "the project you have up" is the
/// obvious target — no setup step needed); and finally, if there's only
/// one project at all, that one. Short-circuits before touching disk or
/// docker once an explicit or sticky project answers the question.
pub fn resolve_project(explicit: Option<String>) -> Result<(String, Option<ProjectSource>)> {
    if let Some(name) = explicit {
        return Ok((name, None));
    }
    if let Some(name) = paths::read_current_project()? {
        return Ok((name, Some(ProjectSource::Sticky)));
    }
    let all = paths::all_project_names()?;
    let running = running_projects(&all)?;
    let (name, source) = pick_running_or_sole_project(&all, &running)?;
    Ok((name, Some(source)))
}

/// `resolve_project`, but announces the result — used by commands (like
/// `moor keel`/`moor view`) that can act on a project without it ever
/// appearing in the command line, so it's always visible which container
/// is about to be touched, not just inferred silently.
pub fn resolve_project_announced(explicit: Option<String>) -> Result<String> {
    let (name, source) = resolve_project(explicit)?;
    match source {
        Some(source) => println!("==> project: {name} ({})", source.describe()),
        None => println!("==> project: {name}"),
    }
    Ok(name)
}

/// Interactively confirm and create a private GitHub repo named `name`,
/// returning its "owner/repo" slug. `Ok(None)` means the operator
/// declined the confirmation prompt — callers should treat that as "stop
/// here, don't scaffold anything," not as an error. Shared by `new` and
/// `import` so the confirm-then-create-then-report flow can't drift
/// between the two.
pub fn create_github_repo_interactive(name: &str) -> Result<Option<String>> {
    print!(
        "About to run `gh repo create {name} --private` on your GitHub account. Continue? [y/N] "
    );
    io::stdout().flush().ok();
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !answer.trim().eq_ignore_ascii_case("y") {
        return Ok(None);
    }

    let (status, whoami) = crate::proc::run_capture("gh", &["api", "user", "-q", ".login"])?;
    crate::proc::require_success("gh api user", status)?;
    let owner = whoami.trim();
    let repo_slug = format!("{owner}/{name}");

    let status = crate::proc::run_inherit("gh", &["repo", "create", &repo_slug, "--private"])?;
    crate::proc::require_success("gh repo create", status)?;
    Ok(Some(repo_slug))
}

/// Re-render compose.yml from the current manifest and bring the pair up.
/// Called by both `new` and `up` so a hand-edited moor.yaml always
/// takes effect on the next start, not just at creation time.
pub fn compose_up(name: &str) -> Result<()> {
    let m = Manifest::load(&paths::manifest_path(name)?)?;
    // Anything not already in this process's own environment gets a
    // chance to resolve from the Keychain before we hand off to `docker
    // compose`, which reads ${VAR} substitutions from its parent
    // process's environment — see secrets.rs.
    crate::secrets::resolve_into_env(name, &m.secrets);
    let rendered = compose::render(&m);
    let compose_path = paths::compose_path(name)?;
    std::fs::write(&compose_path, rendered)
        .with_context(|| format!("writing {}", compose_path.display()))?;

    let compose_path_str = compose_path.to_string_lossy().to_string();
    let status = crate::proc::run_inherit(
        "docker",
        &["compose", "-p", name, "-f", &compose_path_str, "up", "-d"],
    )?;
    crate::proc::require_success("docker compose up", status)?;
    sync_git_identity(&m);
    Ok(())
}

/// Give the sandbox's own git identity a real name, sourced from the
/// *host's* own `git config`, instead of leaving it unset and letting
/// keel's own identity lookup fall all the way through to "unknown" in
/// things like `keel approve`'s audit record (see
/// docs/decisions/0007-git-identity.md). Always local (never
/// `--global`): the container's rootfs is read-only outside its mounted
/// volumes, so only the repo-local `.git/config` — on the writable
/// workspace volume — can actually be written to. Best-effort and
/// silent: does nothing if there's no repo yet (called too early, before
/// `keel init`/clone/import has created one — callers that know a repo
/// now exists call this again afterward) or if the host itself has no
/// git identity configured.
pub fn sync_git_identity(m: &Manifest) {
    let container = m.sandbox_container();
    for key in ["user.name", "user.email"] {
        if let Ok((status, out)) = crate::proc::run_capture("git", &["config", "--get", key]) {
            let value = out.trim();
            if status.success() && !value.is_empty() {
                let _ = crate::proc::run_capture(
                    "docker",
                    &["exec", &container, "git", "config", key, value],
                );
            }
        }
    }
}

pub fn compose_down(name: &str) -> Result<()> {
    let compose_path = paths::compose_path(name)?;
    if !compose_path.exists() {
        anyhow::bail!("no compose.yml for project '{name}' — has `moor new` been run?");
    }
    let compose_path_str = compose_path.to_string_lossy().to_string();
    let status = crate::proc::run_inherit(
        "docker",
        &["compose", "-p", name, "-f", &compose_path_str, "down"],
    )?;
    crate::proc::require_success("docker compose down", status)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn one_running_project_wins_even_with_others_present() {
        let all = names(&["banner-demo", "isolator-sample-app", "keel"]);
        let running = names(&["keel"]);
        let (name, source) = pick_running_or_sole_project(&all, &running).unwrap();
        assert_eq!(name, "keel");
        assert!(matches!(source, ProjectSource::RunningUnique));
    }

    #[test]
    fn sole_project_wins_when_none_are_running() {
        let all = names(&["only-project"]);
        let running = names(&[]);
        let (name, source) = pick_running_or_sole_project(&all, &running).unwrap();
        assert_eq!(name, "only-project");
        assert!(matches!(source, ProjectSource::SoleProject));
    }

    #[test]
    fn no_projects_at_all_is_an_error() {
        let err = pick_running_or_sole_project(&[], &[]).unwrap_err();
        assert!(err.to_string().contains("moor new"));
    }

    #[test]
    fn multiple_running_projects_is_ambiguous() {
        let all = names(&["banner-demo", "keel"]);
        let running = names(&["banner-demo", "keel"]);
        let err = pick_running_or_sole_project(&all, &running).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("banner-demo") && msg.contains("keel"));
        assert!(msg.contains("--project") || msg.contains("moor use"));
    }

    #[test]
    fn multiple_projects_none_running_is_ambiguous() {
        let all = names(&["banner-demo", "isolator-sample-app", "keel"]);
        let err = pick_running_or_sole_project(&all, &[]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("none is up"));
        assert!(msg.contains("moor up"));
    }

    #[test]
    fn describes_each_source_distinctly() {
        let sticky = ProjectSource::Sticky.describe();
        let running = ProjectSource::RunningUnique.describe();
        let sole = ProjectSource::SoleProject.describe();
        assert_ne!(sticky, running);
        assert_ne!(running, sole);
        assert_ne!(sticky, sole);
    }
}
