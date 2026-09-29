use crate::{audit, manifest, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};

pub fn run(name: &str, image: &str, github: bool) -> Result<()> {
    manifest::validate_name(name)?;

    let dir = paths::project_dir(name)?;
    if dir.exists() {
        anyhow::bail!("project '{name}' already exists at {}", dir.display());
    }

    let mut m = Manifest::new(name, image);

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
        let clone_url = format!("https://github.com/{repo}.git");
        println!("==> cloning {clone_url} into the sandbox workspace volume");
        // Freshly created repos are private, so cloning needs a credential.
        // GITHUB_TOKEN (if the operator exported it before `moor new`)
        // is already sitting in the *container's own* environment via the
        // compose template's ${GITHUB_TOKEN:-} substitution — so this
        // reads it there, inside the container's shell, rather than
        // passing the token as a CLI argument to `docker exec` (which
        // would put it in this process's own argv and, from there, in
        // `docker inspect`/`ps` output on the host).
        let credential_helper =
            "!f() { echo username=x-access-token; echo \"password=$GITHUB_TOKEN\"; }; f";
        let status = proc::run_inherit(
            "docker",
            &[
                "exec",
                &m.sandbox_container(),
                "git",
                "-c",
                &format!("credential.helper={credential_helper}"),
                "clone",
                &clone_url,
                ".",
            ],
        )?;
        audit::log_exec(
            name,
            &m,
            "exec",
            &["git".into(), "clone".into(), clone_url],
            status.code(),
        )?;
        if !status.success() {
            println!(
                "note: clone failed — if '{repo}' is private, export GITHUB_TOKEN (e.g. `export GITHUB_TOKEN=$(gh auth token)`) before `moor new`/`up` so the sandbox can authenticate."
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
