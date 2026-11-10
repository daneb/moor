//! `moor sync`: keep the sandbox's trunk branch current with origin
//! without disturbing whatever branch — and whatever uncommitted
//! `.keel/specs/`/`.keel/runs/` pipeline state — happens to be checked
//! out. The fetch itself always runs on the host, with the host's own
//! `gh`-resolved credential (ADR-0014), never as a `docker exec` into the
//! sandbox, and crosses in only as a `git bundle` stream, the same
//! mechanism `moor pull` uses. See
//! docs/decisions/0017-sandbox-trunk-sync.md.

use crate::{audit, commands::pull_cmd::https_github_url, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Run `argv` in the sandbox, capturing combined output, logged like any
/// exec. Returns (success, output).
fn exec(name: &str, m: &Manifest, argv: &[&str]) -> Result<(bool, String)> {
    let container = m.sandbox_container();
    let mut args = vec!["exec", container.as_str()];
    args.extend_from_slice(argv);
    let (status, out) = proc::run_capture_combined("docker", &args)?;
    let logged: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
    audit::log_exec(name, m, "sync", &logged, status.code())?;
    Ok((status.success(), out))
}

/// `origin` rewritten to its HTTPS GitHub URL, or the refusal text
/// (AC-5): sync never silently rewrites some other host.
fn validate_origin(origin: &str) -> Result<String, String> {
    https_github_url(origin).ok_or_else(|| {
        format!(
            "moor sync only knows how to sync from a github.com remote over HTTPS; origin is:\n  {origin}"
        )
    })
}

/// The host credential sync fetches with, or the instruction to get one
/// (AC-6) — checked before any fetch or ref update runs.
fn require_credential(token: Option<&str>) -> Result<&str, String> {
    token.ok_or_else(|| {
        "no GitHub credential on the host to sync with — run `gh auth login`, then try `moor sync` again."
            .to_string()
    })
}

/// The host git argv that discovers origin's default branch (AC-1): a
/// plain host invocation, never `docker`/`exec`.
fn symref_argv(https: &str) -> Vec<String> {
    vec![
        "ls-remote".to_string(),
        "--symref".to_string(),
        https.to_string(),
        "HEAD".to_string(),
    ]
}

/// Parse `git ls-remote --symref <url> HEAD`'s output for the branch
/// origin's HEAD points at, e.g. "ref: refs/heads/master\tHEAD" ->
/// "master".
fn parse_trunk_name(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let rest = l.strip_prefix("ref: refs/heads/")?;
        rest.split_whitespace().next().map(str::to_string)
    })
}

/// The host git argv that fetches `trunk` from `https` into `scratch`
/// (AC-1): a plain host invocation, never `docker`/`exec`.
fn fetch_argv(scratch: &str, https: &str, trunk: &str) -> Vec<String> {
    vec![
        "-C".to_string(),
        scratch.to_string(),
        "fetch".to_string(),
        https.to_string(),
        trunk.to_string(),
    ]
}

/// Where a fetched bundle lands inside the sandbox's repo: `Branch`
/// advances trunk's own ref (AC-2) — git itself refuses this while trunk
/// is the checked-out branch, so it never races a dirty checkout.
/// `Tracking` advances trunk's remote-tracking ref instead, safe no
/// matter what's checked out; used by `moor doctor`'s refresh (AC-7).
pub(crate) enum TrunkRef {
    Branch,
    Tracking,
}

impl TrunkRef {
    fn path(&self, trunk: &str) -> String {
        match self {
            TrunkRef::Branch => format!("refs/heads/{trunk}"),
            TrunkRef::Tracking => format!("refs/remotes/origin/{trunk}"),
        }
    }
}

/// When trunk itself is checked out, sync may only fast-forward it, and
/// only with a clean tree (AC-4): a dirty tree moves no ref and names
/// what's in the way.
fn ff_only_precondition(trunk: &str, porcelain: &str) -> Result<(), String> {
    let entries: Vec<&str> = porcelain
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if entries.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{trunk} is checked out with {} uncommitted change(s); sync won't fast-forward over them:\n  {}",
            entries.len(),
            entries.join("\n  ")
        ))
    }
}

/// The report line `moor sync` prints (AC-9).
fn report_line(trunk: &str, moved: u32) -> String {
    if moved == 0 {
        format!("{trunk} is already up to date with origin — no ref moved.")
    } else {
        format!("{trunk} advanced {moved} commit(s) to match origin.")
    }
}

/// Origin's HTTPS URL, the host credential, and origin's trunk branch
/// name — resolved before any fetch runs (AC-5/AC-6), shared by `moor
/// sync` and doctor's refresh.
fn preflight(name: &str, m: &Manifest) -> Result<(String, String, String)> {
    let (ok, origin) = exec(name, m, &["git", "remote", "get-url", "origin"])?;
    if !ok || origin.trim().is_empty() {
        anyhow::bail!("the sandbox's repository has no `origin` remote to sync from");
    }
    let https = validate_origin(origin.trim()).map_err(|e| anyhow::anyhow!(e))?;
    let token = require_credential(super::host_github_token().as_deref())
        .map_err(|e| anyhow::anyhow!(e))?
        .to_string();
    let argv = symref_argv(&https);
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (ok, out) = super::run_git_as_host(&token, &args)?;
    if !ok {
        anyhow::bail!("couldn't discover origin's default branch:\n{out}");
    }
    let trunk = parse_trunk_name(&out)
        .ok_or_else(|| anyhow::anyhow!("couldn't parse origin's default branch from:\n{out}"))?;
    Ok((https, token, trunk))
}

/// Fetch `trunk` from `https` on the host (AC-1) into a scratch
/// directory, bundled as `sync.bundle`. The caller removes the returned
/// directory once done with the bundle.
fn fetch_bundle(https: &str, trunk: &str, token: &str) -> Result<(PathBuf, PathBuf)> {
    let dir = std::env::temp_dir().join(format!(
        "moor-sync-{}-{}",
        std::process::id(),
        trunk.replace('/', "-")
    ));
    std::fs::create_dir_all(&dir).context("creating a scratch directory for the sync")?;
    let scratch = dir.join("scratch");
    let scratch_str = scratch.to_string_lossy().to_string();
    let result: Result<PathBuf> = (|| {
        proc::require_success(
            "git init (scratch)",
            proc::run_capture("git", &["init", "-q", &scratch_str])?.0,
        )?;
        let argv = fetch_argv(&scratch_str, https, trunk);
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let (ok, out) = super::run_git_as_host(token, &args)?;
        if !ok {
            anyhow::bail!("git fetch failed:\n{out}");
        }
        let bundle_path = dir.join("sync.bundle");
        let bundle_str = bundle_path.to_string_lossy().to_string();
        proc::require_success(
            "git bundle create",
            proc::run_capture(
                "git",
                &[
                    "-C",
                    &scratch_str,
                    "bundle",
                    "create",
                    &bundle_str,
                    "FETCH_HEAD",
                ],
            )?
            .0,
        )?;
        Ok(bundle_path)
    })();
    match result {
        Ok(bundle_path) => Ok((dir, bundle_path)),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

/// Land a fetched bundle inside the sandbox at `dest_ref` via a plain
/// fetch refspec (AC-2/AC-3): git itself refuses this when `dest_ref` is
/// the currently checked-out branch, never a checkout/merge/reset.
/// Returns the ref's old (possibly nonexistent) and new commit.
fn land_at_ref(
    name: &str,
    m: &Manifest,
    bundle_path: &Path,
    trunk: &str,
    dest_ref: &str,
) -> Result<(String, String)> {
    let (_, old_sha) = exec(name, m, &["git", "rev-parse", dest_ref])?;
    let old_sha = old_sha.trim().to_string();
    let container = m.sandbox_container();
    super::stream_into_sandbox(&container, bundle_path, "/tmp/sync.bundle")?;
    let refspec = format!("{trunk}:{dest_ref}");
    let (ok, out) = exec(
        name,
        m,
        &["git", "fetch", "-q", "/tmp/sync.bundle", &refspec],
    )?;
    let _ = proc::run_capture(
        "docker",
        &["exec", &container, "rm", "-f", "/tmp/sync.bundle"],
    );
    if !ok {
        anyhow::bail!("landing the synced bundle at {dest_ref} failed:\n{out}");
    }
    let (_, new_sha) = exec(name, m, &["git", "rev-parse", dest_ref])?;
    Ok((old_sha, new_sha.trim().to_string()))
}

/// Commits between `old_sha` and `new_sha` in the sandbox's repo; `""` for
/// `old_sha` means the ref didn't exist before, so every reachable commit
/// counts as moved.
fn commits_between(name: &str, m: &Manifest, old_sha: &str, new_sha: &str) -> Result<u32> {
    if old_sha == new_sha {
        return Ok(0);
    }
    let range = if old_sha.is_empty() {
        new_sha.to_string()
    } else {
        format!("{old_sha}..{new_sha}")
    };
    let (_, count) = exec(name, m, &["git", "rev-list", "--count", &range])?;
    Ok(count.trim().parse().unwrap_or(0))
}

/// Fetch trunk on the host and land it at `kind`'s ref inside `name`'s
/// sandbox (AC-1/AC-2/AC-7). Returns the trunk name and how many commits
/// it moved; 0 means already up to date.
pub(crate) fn fetch_trunk(name: &str, m: &Manifest, kind: TrunkRef) -> Result<(String, u32)> {
    let (https, token, trunk) = preflight(name, m)?;
    let (dir, bundle_path) = fetch_bundle(&https, &trunk, &token)?;
    let dest_ref = kind.path(&trunk);
    let result = land_at_ref(name, m, &bundle_path, &trunk, &dest_ref)
        .and_then(|(old, new)| commits_between(name, m, &old, &new));
    let _ = std::fs::remove_dir_all(&dir);
    Ok((trunk, result?))
}

/// `moor sync`: fast-forward trunk onto origin when it's checked out
/// (AC-4, refused on a dirty tree), or otherwise advance its local
/// branch ref directly, never touching HEAD, the index, or the working
/// tree (AC-2/AC-3).
pub fn run(explicit: Option<String>) -> Result<()> {
    let name = super::resolve_project_announced(explicit)?;
    let m = Manifest::load(&paths::manifest_path(&name)?)?;
    crate::secrets::resolve_into_env(&name, &m.secrets);

    if super::running_projects(std::slice::from_ref(&name))?.is_empty() {
        anyhow::bail!("{name} isn't running.\n\n  Next:  moor up {name}");
    }

    let (https, token, trunk) = preflight(&name, &m)?;
    let (_, head) = exec(&name, &m, &["git", "rev-parse", "--abbrev-ref", "HEAD"])?;
    let head = head.trim().to_string();

    let moved = if head == trunk {
        let (_, porcelain) = exec(&name, &m, &["git", "status", "--porcelain"])?;
        ff_only_precondition(&trunk, &porcelain).map_err(|e| anyhow::anyhow!(e))?;
        let (_, old_sha) = exec(&name, &m, &["git", "rev-parse", "HEAD"])?;
        let old_sha = old_sha.trim().to_string();
        let (dir, bundle_path) = fetch_bundle(&https, &trunk, &token)?;
        let container = m.sandbox_container();
        let result: Result<u32> = (|| {
            super::stream_into_sandbox(&container, &bundle_path, "/tmp/sync.bundle")?;
            let (ok, out) = exec(
                &name,
                &m,
                &["git", "fetch", "-q", "/tmp/sync.bundle", &trunk],
            )?;
            let _ = proc::run_capture(
                "docker",
                &["exec", &container, "rm", "-f", "/tmp/sync.bundle"],
            );
            if !ok {
                anyhow::bail!("fetching the synced bundle into the sandbox failed:\n{out}");
            }
            let (_, new_sha) = exec(&name, &m, &["git", "rev-parse", "FETCH_HEAD"])?;
            let new_sha = new_sha.trim().to_string();
            let moved = commits_between(&name, &m, &old_sha, &new_sha)?;
            if moved > 0 {
                let (ok, out) = exec(&name, &m, &["git", "merge", "--ff-only", "FETCH_HEAD"])?;
                if !ok {
                    anyhow::bail!("fast-forwarding {trunk} failed:\n{out}");
                }
            }
            Ok(moved)
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result?
    } else {
        let (dir, bundle_path) = fetch_bundle(&https, &trunk, &token)?;
        let dest_ref = TrunkRef::Branch.path(&trunk);
        let result = land_at_ref(&name, &m, &bundle_path, &trunk, &dest_ref)
            .and_then(|(old, new)| commits_between(&name, &m, &old, &new));
        let _ = std::fs::remove_dir_all(&dir);
        result?
    };

    println!("{}", report_line(&trunk, moved));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_runs_on_host() {
        let argv = fetch_argv(
            "/tmp/moor-sync-scratch",
            "https://github.com/daneb/keel.git",
            "master",
        );
        assert_eq!(argv[0], "-C");
        assert_eq!(argv[2], "fetch");
        assert!(argv.iter().any(|a| a.contains("github.com")));
        assert!(!argv.iter().any(|a| a == "docker" || a == "exec"));
    }

    #[test]
    fn updates_trunk_ref_without_touching_worktree() {
        assert_eq!(TrunkRef::Branch.path("master"), "refs/heads/master");
        assert_eq!(
            TrunkRef::Tracking.path("master"),
            "refs/remotes/origin/master"
        );
        // The ref update is a bare refspec fetch, never a verb that
        // touches HEAD, the index, or the working tree.
        let refspec = format!("master:{}", TrunkRef::Branch.path("master"));
        for verb in ["checkout", "switch", "merge", "reset", "restore", "stash"] {
            assert!(!refspec.contains(verb), "refspec must not contain {verb}");
        }
    }

    #[test]
    fn dirty_trunk_refuses_fast_forward() {
        let err = ff_only_precondition("master", " M src/lib.rs\n?? scratch.txt\n").unwrap_err();
        assert!(err.contains("src/lib.rs"));
        assert!(err.contains("scratch.txt"));
        assert!(ff_only_precondition("master", "").is_ok());
        assert!(ff_only_precondition("master", "\n  \n").is_ok());
    }

    #[test]
    fn refuses_non_github_origin() {
        let err = validate_origin("git@gitlab.com:acme/thing.git").unwrap_err();
        assert!(err.contains("github.com"));
        assert!(validate_origin("git@github.com:daneb/moor.git").is_ok());
    }

    #[test]
    fn missing_credential_blocks_before_fetch() {
        let err = require_credential(None).unwrap_err();
        assert!(err.contains("gh auth login"));
        assert!(require_credential(Some("tok")).is_ok());
    }

    #[test]
    fn reports_commits_advanced() {
        assert!(report_line("master", 0).contains("up to date"));
        let msg = report_line("master", 3);
        assert!(msg.contains("master"));
        assert!(msg.contains('3'));
    }

    #[test]
    fn parses_trunk_from_symref_output() {
        let out = "ref: refs/heads/master\tHEAD\nabc123\tHEAD\n";
        assert_eq!(parse_trunk_name(out).as_deref(), Some("master"));
    }
}
