use crate::{audit, manifest, manifest::Agent, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};

pub fn run(name: &str, image: &str, agent: Agent, github: bool) -> Result<()> {
    manifest::validate_name(name)?;

    let dir = paths::project_dir(name)?;
    if dir.exists() {
        anyhow::bail!("project '{name}' already exists at {}", dir.display());
    }

    // Compose the chosen agent with the (language) image: Claude leaves it
    // as-is, Copilot selects the matching moor/copilot-<lang> layer.
    let image = manifest::resolve_image(image, agent);
    let mut m = Manifest::new(name, &image);
    m.agent = agent;
    // A kiro project records the Kiro credential env-var names; other
    // agents are left with the default secret set.
    m.declare_agent_secrets();

    if github {
        match super::create_github_repo_interactive(name)? {
            Some(slug) => m.github_repo = Some(slug),
            None => {
                println!("Aborted — no GitHub repo created, no project scaffolded.");
                return Ok(());
            }
        }
    }

    paths::ensure_project_dirs(name)?;
    m.save(&paths::manifest_path(name)?)
        .context("writing moor.yaml")?;

    println!("==> starting sandbox + egress for '{name}'");
    super::compose_up(name)?;

    if let Some(repo) = &m.github_repo {
        let clone_url = github_clone_url(repo);
        println!("==> cloning {clone_url} into the sandbox workspace volume");
        // Freshly created repos are private, so cloning needs a credential —
        // since ADR-0009 that happens on the host, with the host's own `gh`
        // credential: the sandbox never sees a token, in its environment
        // or anywhere else.
        let ok = clone_into_sandbox(name, &m, &clone_url)?;
        if !ok {
            println!(
                "note: clone failed — if '{repo}' is private, run `gh auth login` on this Mac so the host can authenticate, then `moor new`/`up` again."
            );
        }
    }

    println!("==> setting the workspace up for specs");
    // Captured, not shown: the pipeline's own setup output ends with its own
    // "Next:" list of commands, which would compete with moor's. Shown only
    // when setup fails, since then it's what explains why.
    let status =
        proc::run_capture_combined("docker", &["exec", &m.sandbox_container(), "keel", "init"]);
    match status {
        Ok((s, out)) => {
            audit::log_exec(name, &m, "exec", &["keel".into(), "init".into()], s.code())?;
            if !s.success() {
                println!("{out}");
                println!(
                    "note: workspace setup did not exit cleanly — check with `moor shell {name}`"
                );
            }
        }
        Err(e) => println!("note: could not set the workspace up automatically: {e}"),
    }

    // A build is judged by its diff against a commit, so the workspace has
    // to be a git repository with at least one commit before the first
    // build. Neither setup step guarantees that: without --github nothing
    // runs `git init`, and a new GitHub repo clones empty.
    ensure_repo(name, &m)?;
    // Identity first, so the first commit is attributed to the operator.
    super::sync_git_identity(&m);
    ensure_first_commit(name, &m)?;

    println!(
        "\n'{name}' is up. Manifest: {}\n\n{}",
        paths::manifest_path(name)?.display(),
        super::next_cmd::next_hint(name)
    );
    Ok(())
}

/// `https://github.com/<owner>/<repo>.git` for `repo` ("owner/repo").
fn github_clone_url(repo: &str) -> String {
    format!("https://github.com/{repo}.git")
}

/// The host `git` argv that clones `url` into `scratch` (AC-8) — a plain
/// host invocation, never `docker`/`exec`, so the credential it runs with
/// never has to cross into the sandbox.
fn clone_argv(scratch: &str, url: &str) -> Vec<String> {
    vec!["clone".to_string(), url.to_string(), scratch.to_string()]
}

/// Clone `url` on the host with the host's own GitHub credential (ADR-0009),
/// then stream the result into the sandbox as a git bundle (mirroring
/// `import_cmd`'s bundle-and-stream, but sourced from a host-side clone
/// instead of an existing local repo). Returns `false` (no error) when
/// there's no host credential to clone with, so the caller can print its
/// own hint.
fn clone_into_sandbox(name: &str, m: &Manifest, clone_url: &str) -> Result<bool> {
    let Some(token) = super::host_github_token() else {
        return Ok(false);
    };
    let dir = std::env::temp_dir().join(format!("moor-new-clone-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).context("creating a scratch directory for the clone")?;
    let cleanup = || {
        let _ = std::fs::remove_dir_all(&dir);
    };
    let scratch = dir.join("scratch");
    let scratch_str = scratch.to_string_lossy().to_string();

    let result: Result<bool> = (|| {
        let argv = clone_argv(&scratch_str, clone_url);
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let (ok, out) = super::run_git_as_host(&token, &args)?;
        if !ok {
            println!("{out}");
            return Ok(false);
        }

        let bundle_path = dir.join("new.bundle");
        let bundle_str = bundle_path.to_string_lossy().to_string();
        proc::require_success(
            "git bundle create",
            proc::run_capture(
                "git",
                &["-C", &scratch_str, "bundle", "create", &bundle_str, "--all"],
            )?
            .0,
        )?;

        let container = m.sandbox_container();
        super::stream_into_sandbox(&container, &bundle_path, "/tmp/new.bundle")?;
        let status = proc::run_inherit(
            "docker",
            &["exec", &container, "git", "clone", "/tmp/new.bundle", "."],
        )?;
        audit::log_exec(
            name,
            m,
            "exec",
            &["git".into(), "clone".into(), clone_url.to_string()],
            status.code(),
        )?;
        proc::require_success("git clone (bundle) in sandbox", status)?;
        let _ = proc::run_capture(
            "docker",
            &["exec", &container, "rm", "-f", "/tmp/new.bundle"],
        );

        let status = proc::run_inherit(
            "docker",
            &[
                "exec", &container, "git", "remote", "set-url", "origin", clone_url,
            ],
        )?;
        proc::require_success("git remote set-url origin", status)?;
        Ok(true)
    })();
    cleanup();
    result
}

/// Runs `argv` in the sandbox, quietly, logged like any exec.
fn exec_quiet(name: &str, m: &Manifest, argv: &[&str]) -> Result<bool> {
    let container = m.sandbox_container();
    let mut args = vec!["exec", container.as_str()];
    args.extend_from_slice(argv);
    let (status, _) = proc::run_capture_combined("docker", &args)?;
    let logged: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
    audit::log_exec(name, m, "exec", &logged, status.code())?;
    Ok(status.success())
}

fn ensure_repo(name: &str, m: &Manifest) -> Result<()> {
    if !exec_quiet(name, m, &["git", "rev-parse", "--git-dir"])? {
        println!("==> making the workspace a git repository");
        if !exec_quiet(name, m, &["git", "init", "-q", "-b", "main"])? {
            println!("note: `git init` failed in the workspace — check with `moor shell {name}`");
        }
    }
    Ok(())
}

fn ensure_first_commit(name: &str, m: &Manifest) -> Result<()> {
    if exec_quiet(
        name,
        m,
        &["git", "rev-parse", "--verify", "--quiet", "HEAD"],
    )? {
        return Ok(());
    }
    println!("==> committing the starting point, so the first build has a base");
    let committed = exec_quiet(name, m, &["git", "add", "-A"])?
        && exec_quiet(
            name,
            m,
            &[
                "git",
                "commit",
                "-q",
                "-m",
                "Set up the workspace for specs",
            ],
        )?;
    if !committed {
        println!(
            "note: could not make the first commit (is `git config user.name`/`user.email` set on this Mac?) — builds need one; make it with `moor shell {name}`"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_clone_uses_host_credential() {
        // AC-8: cloning runs on the host (plain `git clone`), never a
        // `docker exec` into the sandbox — the credential it uses with it
        // stays on the host.
        let argv = clone_argv("/tmp/moor-new-scratch", &github_clone_url("daneb/keel"));
        assert_eq!(argv[0], "clone");
        assert!(argv.iter().any(|a| a.contains("github.com")));
        assert!(!argv.iter().any(|a| a == "docker" || a == "exec"));
    }
}
