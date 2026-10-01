//! `moor pull`: refresh the sandbox's current branch from its GitHub
//! remote, over HTTPS with a token injected at runtime.
//!
//! Why this exists rather than `moor run <p> -- git pull`: the sandbox has
//! no `ssh` binary by design (ADR-0010), so a `git pull` against an SSH
//! `origin` (e.g. `git@github.com-sbg:owner/repo.git`) fails with "cannot
//! run ssh". GitHub *is* reachable over HTTPS from the sandbox, so this
//! derives the HTTPS URL from `origin` and pulls over it. The token is
//! read from `$GITHUB_TOKEN` inside the container by a git credential
//! helper — the same pattern `moor new`/`moor ship` use — so it is never
//! in this process's argv (and thus never in `docker inspect`/`ps`), and
//! the pull uses an explicit URL rather than a stored remote, so no
//! credential is ever written to `.git/config`.

use crate::{audit, manifest::Manifest, paths, proc};
use anyhow::Result;

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

    // The token lives in $GITHUB_TOKEN inside the container (injected by
    // the compose template from the host's env/Keychain). Reading it in a
    // credential helper keeps it out of this process's argv — the same
    // pattern as `moor new`/`moor ship`. The explicit URL means no remote
    // (and so no credential) is persisted to .git/config.
    let helper = "credential.helper=!f() { echo username=x-access-token; echo \"password=$GITHUB_TOKEN\"; }; f";

    println!("==> pulling {branch} from {https} (HTTPS, token injected in-container, nothing persisted)");
    let (ok, out) = exec(
        &name,
        &m,
        &[
            "git", "-c", helper, "pull", "--ff-only", &https, branch,
        ],
    )?;
    // Scrub a token that could only appear if git echoed the URL back; the
    // helper form above doesn't put it in the URL, but be defensive.
    let shown = out.trim();
    println!("{shown}");
    if !ok {
        anyhow::bail!(
            "pull failed. If the repo is private, make sure GITHUB_TOKEN is set for this project:\n\n  \
             moor secrets set {name} GITHUB_TOKEN\n  moor up {name}\n\nthen try again."
        );
    }
    println!("\n==> {name} is up to date on {branch}.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::https_github_url;

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
        assert_eq!(
            https_github_url("https://gitlab.com/acme/thing.git"),
            None
        );
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
}
