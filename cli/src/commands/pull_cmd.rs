//! `moor pull`: refresh the sandbox's current branch from its GitHub
//! remote, over HTTPS with the host's own credential (ADR-0009).
//!
//! Why this exists rather than `moor run <p> -- git pull`: the sandbox has
//! no `ssh` binary by design (ADR-0010), so a `git pull` against an SSH
//! `origin` (e.g. `git@github.com-sbg:owner/repo.git`) fails with "cannot
//! run ssh". GitHub *is* reachable over HTTPS, so this derives the HTTPS
//! URL from `origin` and fetches over it — but the fetch itself runs on
//! the host, with the host's own `gh`-resolved credential: the sandbox
//! never sees a token. The fetched commits are streamed into the sandbox
//! as a git bundle and merged there with `--ff-only`.

use crate::{audit, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};

/// The https://github.com/owner/repo.git URL for a GitHub remote, whatever
/// form `origin` takes:
///   - scp-style SSH, incl. a host alias:  git@github.com-sbg:owner/repo.git
///   - ssh:// URL:                         ssh://git@github.com/owner/repo.git
///   - already HTTPS:                      https://github.com/owner/repo.git
///
/// Only `github.com` (or an SSH host alias whose host part contains
/// `github.com`) is accepted; anything else returns None so we never
/// silently rewrite some other host. `owner`/`repo` must have a safe
/// shape, matching `ship_cmd`'s parser.
pub fn https_github_url(origin: &str) -> Option<String> {
    let origin = origin.trim();

    // owner/repo, suffix-trimmed, from the part after the host.
    let path = if let Some(rest) = origin.strip_prefix("https://github.com/") {
        rest.to_string()
    } else if let Some(rest) = origin.strip_prefix("ssh://") {
        // ssh://git@github.com[:port]/owner/repo(.git)
        let rest = rest.split_once('/')?.1;
        rest.to_string()
    } else if let Some((host, rest)) = origin.split_once(':') {
        // scp-style: [user@]host:owner/repo(.git). The host may be an SSH
        // alias like `github.com-sbg`; accept it only if its label chain
        // includes a literal `github.com`.
        let host = host.rsplit('@').next().unwrap_or(host);
        if host != "github.com" && !host.starts_with("github.com") {
            return None;
        }
        rest.to_string()
    } else {
        return None;
    };

    let path = path.strip_suffix(".git").unwrap_or(&path);
    let (owner, repo) = path.split_once('/')?;
    // repo may carry a trailing path only in malformed input; take the
    // first two segments and validate their shape.
    let repo = repo.split('/').next().unwrap_or(repo);
    let safe = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    (safe(owner) && safe(repo)).then(|| format!("https://github.com/{owner}/{repo}.git"))
}

/// Run `argv` in the sandbox, capturing combined output, logged like any
/// exec. Returns (success, output).
fn exec(name: &str, m: &Manifest, argv: &[&str]) -> Result<(bool, String)> {
    let container = m.sandbox_container();
    let mut args = vec!["exec", container.as_str()];
    args.extend_from_slice(argv);
    let (status, out) = proc::run_capture_combined("docker", &args)?;
    let logged: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
    audit::log_exec(name, m, "pull", &logged, status.code())?;
    Ok((status.success(), out))
}

pub fn run(explicit: Option<String>) -> Result<()> {
    let name = super::resolve_project_announced(explicit)?;
    let m = Manifest::load(&paths::manifest_path(&name)?)?;
    crate::secrets::resolve_into_env(&name, &m.secrets);

    if super::running_projects(std::slice::from_ref(&name))?.is_empty() {
        anyhow::bail!("{name} isn't running.\n\n  Next:  moor up {name}");
    }

    // The remote and the current branch, from the sandbox.
    let (ok, origin) = exec(&name, &m, &["git", "remote", "get-url", "origin"])?;
    if !ok || origin.trim().is_empty() {
        anyhow::bail!("the sandbox's repository has no `origin` remote to pull from");
    }
    let origin = origin.trim();
    let Some(https) = https_github_url(origin) else {
        anyhow::bail!(
            "moor pull only knows how to pull from a github.com remote over HTTPS; origin is:\n  {origin}"
        );
    };
    let (ok, branch) = exec(&name, &m, &["git", "rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = branch.trim();
    if !ok || branch.is_empty() || branch == "HEAD" {
        anyhow::bail!("the sandbox's repository isn't on a branch to pull");
    }

    let Some(token) = super::host_github_token() else {
        anyhow::bail!(
            "no GitHub credential on the host to pull with — run `gh auth login`, then try `moor pull` again."
        );
    };

    println!("==> pulling {branch} from {https} (fetched on the host, merged in the sandbox)");
    pull_branch(&name, &m, &https, branch, &token)?;
    println!("
==> {name} is up to date on {branch}.");
    Ok(())
}

/// The host `git` argv that fetches `branch` from `https` into `scratch`
/// (AC-9) — a plain host invocation, never `docker`/`exec`.
fn fetch_argv(scratch: &str, https: &str, branch: &str) -> Vec<String> {
    vec![
        "-C".to_string(),
        scratch.to_string(),
        "fetch".to_string(),
        https.to_string(),
        branch.to_string(),
    ]
}

/// Fetch `branch` from `https` on the host with `token` (ADR-0009), then
/// stream the fetched commits into the sandbox as a git bundle and merge
/// them there with `--ff-only` — a pull never runs `git pull`/`fetch`
/// directly inside the container, so no credential ever has to be there.
fn pull_branch(name: &str, m: &Manifest, https: &str, branch: &str, token: &str) -> Result<()> {
    let dir = std::env::temp_dir().join(format!("moor-pull-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).context("creating a scratch directory for the pull")?;
    let cleanup = || {
        let _ = std::fs::remove_dir_all(&dir);
    };
    let scratch = dir.join("scratch");
    let scratch_str = scratch.to_string_lossy().to_string();

    let result: Result<()> = (|| {
        proc::require_success("git init (scratch)", proc::run_capture("git", &["init", "-q", &scratch_str])?.0)?;

        let argv = fetch_argv(&scratch_str, https, branch);
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let (ok, out) = super::run_git_as_host(token, &args)?;
        if !ok {
            anyhow::bail!("git fetch failed:
{out}");
        }

        let bundle_path = dir.join("pull.bundle");
        let bundle_str = bundle_path.to_string_lossy().to_string();
        proc::require_success(
            "git bundle create",
            proc::run_capture("git", &["-C", &scratch_str, "bundle", "create", &bundle_str, "FETCH_HEAD"])?.0,
        )?;

        let container = m.sandbox_container();
        super::stream_into_sandbox(&container, &bundle_path, "/tmp/pull.bundle")?;
        let (ok, out) = exec(
            name,
            m,
            &["sh", "-c", "git fetch -q /tmp/pull.bundle FETCH_HEAD && git merge --ff-only FETCH_HEAD"],
        )?;
        let _ = proc::run_capture("docker", &["exec", &container, "rm", "-f", "/tmp/pull.bundle"]);
        if !ok {
            anyhow::bail!("merge failed in the sandbox:
{out}");
        }
        Ok(())
    })();
    cleanup();
    result
}

#[cfg(test)]
mod tests {
    use super::{fetch_argv, https_github_url};

    #[test]
    fn rewrites_scp_style_with_host_alias() {
        assert_eq!(
            https_github_url("git@github.com-sbg:Dane-Balia_sbg/filenet-retriever.git"),
            Some("https://github.com/Dane-Balia_sbg/filenet-retriever.git".to_string())
        );
    }

    #[test]
    fn rewrites_plain_scp_style() {
        assert_eq!(
            https_github_url("git@github.com:daneb/moor.git"),
            Some("https://github.com/daneb/moor.git".to_string())
        );
    }

    #[test]
    fn rewrites_ssh_url_form() {
        assert_eq!(
            https_github_url("ssh://git@github.com/daneb/moor.git"),
            Some("https://github.com/daneb/moor.git".to_string())
        );
    }

    #[test]
    fn passes_through_https() {
        assert_eq!(
            https_github_url("https://github.com/daneb/moor.git"),
            Some("https://github.com/daneb/moor.git".to_string())
        );
    }

    #[test]
    fn adds_git_suffix_when_missing() {
        assert_eq!(
            https_github_url("git@github.com:daneb/moor"),
            Some("https://github.com/daneb/moor.git".to_string())
        );
    }

    #[test]
    fn refuses_non_github_host() {
        assert_eq!(https_github_url("git@gitlab.com:acme/thing.git"), None);
        assert_eq!(https_github_url("https://gitlab.com/acme/thing.git"), None);
    }

    #[test]
    fn refuses_unsafe_owner_or_repo() {
        assert_eq!(https_github_url("git@github.com:ow ner/repo.git"), None);
        assert_eq!(https_github_url("git@github.com:owner/re;po.git"), None);
    }

    #[test]
    fn refuses_garbage() {
        assert_eq!(https_github_url("not a url"), None);
        assert_eq!(https_github_url(""), None);
    }

    #[test]
    fn pull_runs_on_host() {
        let argv = fetch_argv(
            "/tmp/moor-pull-scratch",
            "https://github.com/daneb/keel.git",
            "main",
        );
        assert_eq!(argv[0], "-C");
        assert_eq!(argv[2], "fetch");
        assert!(argv.iter().any(|a| a.contains("github.com")));
        assert!(!argv.iter().any(|a| a == "docker" || a == "exec"));
    }
}
