use crate::{audit, manifest, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};
use std::path::Path;

/// Look at the top level of an existing local project and guess which
/// moor image fits it. Pure — no filesystem side effects beyond the
/// read-only existence checks — so it's unit-testable against a fixture
/// directory without touching Docker.
fn detect_image(from: &Path) -> &'static str {
    if from.join("Cargo.toml").exists() {
        "moor/rust:latest"
    } else if from.join("package.json").exists() {
        "moor/node:latest"
    } else if from.join("pyproject.toml").exists() || from.join("requirements.txt").exists() {
        "moor/python:latest"
    } else {
        "moor/base:latest"
    }
}

/// Parse a git remote URL's "owner/repo" out of the common GitHub URL
/// forms (`https://github.com/owner/repo(.git)`,
/// `git@github.com:owner/repo(.git)`, `ssh://git@github.com/owner/repo(.git)`).
/// Returns `None` for anything else (a non-GitHub remote, GitLab, a local
/// path) — moor's manifest only tracks a `github_repo` field
/// specifically, so a non-GitHub remote is left for the operator to note
/// themselves rather than guessed at.
fn parse_github_repo_slug(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches(".git");
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))?;
    let (owner, repo) = rest.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

pub fn run(name: &str, from: &Path, image: Option<String>, github: bool) -> Result<()> {
    manifest::validate_name(name)?;

    let dir = paths::project_dir(name)?;
    if dir.exists() {
        anyhow::bail!("project '{name}' already exists at {}", dir.display());
    }

    let from = from
        .canonicalize()
        .with_context(|| format!("'{}' does not exist", from.display()))?;
    if !from.join(".git").exists() {
        anyhow::bail!(
            "'{}' is not a git repository (no .git) — moor only imports version-controlled projects",
            from.display()
        );
    }

    let image = image.unwrap_or_else(|| detect_image(&from).to_string());
    println!(
        "==> importing {} as '{name}' (image: {image})",
        from.display()
    );

    let mut m = Manifest::new(name, &image);

    // Preserve the source repo's existing GitHub remote, if it has one —
    // this is the common case for something like keel, which already
    // lives at github.com/<owner>/keel.
    let (status, out) = proc::run_capture(
        "git",
        &["-C", &from.to_string_lossy(), "remote", "get-url", "origin"],
    )?;
    let existing_remote = if status.success() {
        Some(out.trim().to_string())
    } else {
        None
    };
    if let Some(url) = &existing_remote {
        if let Some(slug) = parse_github_repo_slug(url) {
            m.github_repo = Some(slug);
        }
    } else if github {
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

    import_history(name, &m, &from, existing_remote.as_deref())?;
    // `compose_up` tried this before the import above created a repo —
    // retry now that one actually exists.
    super::sync_git_identity(&m);

    println!("==> checking for existing keel configuration");
    let (status, _) = proc::run_capture(
        "docker",
        &[
            "exec",
            &m.sandbox_container(),
            "test",
            "-f",
            ".keel/keel.toml",
        ],
    )?;
    if status.success() {
        println!(
            "   .keel/keel.toml already present — leaving it as-is (not re-running `keel init`)"
        );
        let status = proc::run_inherit(
            "docker",
            &["exec", &m.sandbox_container(), "keel", "status"],
        );
        if let Ok(s) = status {
            audit::log_exec(
                name,
                &m,
                "exec",
                &["keel".into(), "status".into()],
                s.code(),
            )?;
        }
    } else {
        println!("   none found — running `keel init` inside the sandbox");
        let status = proc::run_inherit("docker", &["exec", &m.sandbox_container(), "keel", "init"]);
        match status {
            Ok(s) => {
                audit::log_exec(name, &m, "exec", &["keel".into(), "init".into()], s.code())?;
                if !s.success() {
                    println!("   note: `keel init` did not exit cleanly — check with `moor shell {name}`");
                }
            }
            Err(e) => println!("   note: could not run `keel init` automatically: {e}"),
        }
    }

    println!(
        "\n'{name}' is up, imported from {}. Manifest: {}\n\n{}",
        from.display(),
        paths::manifest_path(name)?.display(),
        super::next_cmd::next_hint(name)
    );
    Ok(())
}

/// Transfer the source repo's full history (`--all`: every branch and
/// tag) into the sandbox's workspace volume via a `git bundle` — the only
/// host content that ever crosses into the container for an import, and
/// a deliberate one-shot copy, not a standing bind mount. The bundle is
/// written to the project's own (host-only) directory, copied in with
/// `docker cp`, cloned from inside the container, then deleted on both
/// sides.
fn import_history(name: &str, m: &Manifest, from: &Path, real_remote: Option<&str>) -> Result<()> {
    let bundle_path = paths::project_dir(name)?.join(".import.bundle");
    let bundle_str = bundle_path.to_string_lossy().to_string();
    let from_str = from.to_string_lossy().to_string();

    println!("==> bundling {} (all branches and tags)", from.display());
    let status = proc::run_inherit(
        "git",
        &["-C", &from_str, "bundle", "create", &bundle_str, "--all"],
    )?;
    proc::require_success("git bundle create", status)?;

    let container = m.sandbox_container();
    println!("==> streaming the bundle into the sandbox and cloning it");
    // Not `docker cp`: the sandbox's root filesystem is read-only, and
    // `docker cp`'s own copy mechanism needs write access it doesn't have
    // even when the destination (/tmp) is itself a writable tmpfs mount.
    // Streaming through `docker exec`'s stdin only ever touches that one
    // writable path.
    let status = proc::run_with_stdin_file(
        "docker",
        &[
            "exec",
            "-i",
            &container,
            "sh",
            "-c",
            "cat > /tmp/import.bundle",
        ],
        &bundle_path,
    )?;
    proc::require_success("streaming import bundle into sandbox", status)?;
    let _ = std::fs::remove_file(&bundle_path);

    let status = proc::run_inherit(
        "docker",
        &[
            "exec",
            &container,
            "git",
            "clone",
            "/tmp/import.bundle",
            ".",
        ],
    )?;
    audit::log_exec(
        name,
        m,
        "exec",
        &["git".into(), "clone".into(), "<imported bundle>".into()],
        status.code(),
    )?;
    proc::require_success("git clone (imported bundle) in sandbox", status)?;

    let _ = proc::run_capture(
        "docker",
        &["exec", &container, "rm", "-f", "/tmp/import.bundle"],
    );

    // `git clone` only checks out a local branch for the bundle's default
    // ref — every other branch lands as a remote-tracking ref under
    // refs/remotes/origin/*, not a local branch. If there's no real
    // remote to keep `origin` pointed at, it gets removed below — and
    // removing a remote deletes its tracking refs with it, which would
    // silently drop every branch but the default one. Materialize them
    // as real local branches first so that's never possible.
    let materialize = "for b in $(git for-each-ref --format='%(refname:short)' refs/remotes/origin/ | grep -v '/HEAD$'); do name=${b#origin/}; git show-ref --verify --quiet \"refs/heads/$name\" || git branch \"$name\" \"$b\"; done";
    let status = proc::run_inherit("docker", &["exec", &container, "sh", "-c", materialize])?;
    proc::require_success("materializing imported branches locally", status)?;

    // The bundle clone leaves `origin` pointing at the now-deleted bundle
    // path inside the container — point it at the real remote if there
    // was one, or drop it if there wasn't, rather than leaving a remote
    // that can only ever fail.
    if let Some(url) = real_remote {
        let status = proc::run_inherit(
            "docker",
            &[
                "exec", &container, "git", "remote", "set-url", "origin", url,
            ],
        )?;
        proc::require_success("git remote set-url origin", status)?;
        println!("   origin set to {url}");
    } else if let Some(slug) = &m.github_repo {
        let url = format!("https://github.com/{slug}.git");
        let status = proc::run_inherit(
            "docker",
            &[
                "exec", &container, "git", "remote", "set-url", "origin", &url,
            ],
        )?;
        proc::require_success("git remote set-url origin", status)?;
        println!("   origin set to {url} (created by --github)");
    } else {
        let _ = proc::run_capture(
            "docker",
            &["exec", &container, "git", "remote", "remove", "origin"],
        );
        println!("   no remote configured — set one with `moor run {name} -- git remote add origin <url>` when ready");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn touch(dir: &Path, file: &str) {
        std::fs::write(dir.join(file), "").unwrap();
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("moor-import-test-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_rust_project() {
        let dir = temp_dir("rust");
        touch(&dir, "Cargo.toml");
        assert_eq!(detect_image(&dir), "moor/rust:latest");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_node_project() {
        let dir = temp_dir("node");
        touch(&dir, "package.json");
        assert_eq!(detect_image(&dir), "moor/node:latest");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_python_project_via_pyproject() {
        let dir = temp_dir("py-pyproject");
        touch(&dir, "pyproject.toml");
        assert_eq!(detect_image(&dir), "moor/python:latest");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_python_project_via_requirements_txt() {
        let dir = temp_dir("py-reqs");
        touch(&dir, "requirements.txt");
        assert_eq!(detect_image(&dir), "moor/python:latest");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn falls_back_to_base_when_nothing_recognized() {
        let dir = temp_dir("empty");
        assert_eq!(detect_image(&dir), "moor/base:latest");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rust_takes_priority_when_multiple_markers_present() {
        // A repo can plausibly have both a Cargo.toml (e.g. a Rust CLI)
        // and a package.json (e.g. its docs site) — pick one consistently
        // rather than leaving it to filesystem iteration order.
        let dir = temp_dir("mixed");
        touch(&dir, "Cargo.toml");
        touch(&dir, "package.json");
        assert_eq!(detect_image(&dir), "moor/rust:latest");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_https_github_url() {
        assert_eq!(
            parse_github_repo_slug("https://github.com/daneb/keel"),
            Some("daneb/keel".to_string())
        );
        assert_eq!(
            parse_github_repo_slug("https://github.com/daneb/keel.git"),
            Some("daneb/keel".to_string())
        );
    }

    #[test]
    fn parses_ssh_github_url_forms() {
        assert_eq!(
            parse_github_repo_slug("git@github.com:daneb/keel.git"),
            Some("daneb/keel".to_string())
        );
        assert_eq!(
            parse_github_repo_slug("ssh://git@github.com/daneb/keel.git"),
            Some("daneb/keel".to_string())
        );
    }

    #[test]
    fn returns_none_for_non_github_remotes() {
        assert_eq!(
            parse_github_repo_slug("https://gitlab.com/daneb/keel.git"),
            None
        );
        assert_eq!(parse_github_repo_slug("/Users/danebalia/Repos/keel"), None);
        assert_eq!(parse_github_repo_slug(""), None);
    }
}
