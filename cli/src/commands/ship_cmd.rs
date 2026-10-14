//! `moor ship`: take a spec whose merge you approved and get it to GitHub.
//! Commits the spec's work on its own branch in the sandbox, then —
//! since ADR-0009 — streams just that branch out as a git bundle and
//! pushes it from the host with the host's own GitHub credential; the
//! sandbox never sees a token. Opens a pull request from the host, and
//! returns the sandbox to its trunk so the next spec starts from a clean
//! tree.
//!
//! Approval stays a separate act (`moor approve`); shipping publishes, so
//! it shows exactly what it will commit and asks first.

use super::next_cmd::Target;
use crate::{audit, guide, manifest, proc};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

/// Paths `git status --porcelain` reports, with a rename's new name.
fn changed_paths(porcelain: &str) -> Vec<String> {
    porcelain
        .lines()
        .filter_map(|l| l.get(3..))
        .map(|p| p.rsplit(" -> ").next().unwrap_or(p))
        .map(|p| p.trim_matches('"').to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Which changed paths belong to `slug`'s change: everything except other
/// specs' folders and runs that belong to other specs. `run_owner` maps a
/// run id to its spec.
fn select(paths: &[String], slug: &str, run_owner: &HashMap<String, String>) -> Vec<String> {
    let own_spec = format!(".keel/specs/{slug}/");
    paths
        .iter()
        .filter(|p| {
            if let Some(rest) = p.strip_prefix(".keel/specs/") {
                return p.starts_with(&own_spec) || !rest.contains('/');
            }
            if let Some(rest) = p.strip_prefix(".keel/runs/") {
                let id = rest.split('/').next().unwrap_or("");
                return run_owner.get(id).is_some_and(|s| s == slug);
            }
            true
        })
        .cloned()
        .collect()
}

/// `owner/repo` from a GitHub remote URL (https or scp-style), if every
/// part has a safe shape.
fn github_repo(url: &str) -> Option<String> {
    let rest = url
        .trim()
        .strip_prefix("https://github.com/")
        .or_else(|| url.trim().strip_prefix("git@github.com:"))?;
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repo) = rest.split_once('/')?;
    let safe = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    (safe(owner) && safe(repo)).then(|| format!("{owner}/{repo}"))
}

/// A branch name as git reports it, accepted only in a plain shape.
fn plain_branch(name: &str) -> Option<String> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name != "HEAD"
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'));
    ok.then(|| name.to_string())
}

/// The spec's title and id from its spec.md, made printable.
fn title_and_id(spec_md: &str, slug: &str) -> (String, Option<String>) {
    let id = super::spec_cmd::read_identity(spec_md).map(|(id, _)| id);
    let title = super::spec_cmd::front_matter(spec_md)
        .and_then(|(_, body)| body.lines().find_map(|l| l.strip_prefix("# ")))
        .map(|t| {
            t.chars()
                .filter(|c| !c.is_control())
                .take(120)
                .collect::<String>()
        })
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| slug.to_string());
    (title, id)
}

fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// The spec to ship: the one named; else the only approved-but-unshipped
/// one; else the active/pinned spec if it is among the unshipped.
///
/// The pin is consulted directly here, not via `Target::active`: once a
/// spec's build is approved it is `complete`, and `pick_active` only
/// returns a pin while the spec is still in progress — so after approval
/// an `active`-based lookup would drop the very spec you just approved,
/// and `moor ship` would say "nothing to ship" while `moor next` points
/// at it. Keying on the pin's presence in `unshipped` keeps the two in
/// agreement.
fn pick(t: &Target, report: &guide::NextReport, named: Option<&str>) -> Result<Option<String>> {
    let unshipped = t.unshipped(report)?;
    Ok(match named {
        Some(n) => Some(n.to_string()),
        None if unshipped.len() == 1 => unshipped.into_iter().next(),
        None => {
            let pin = super::next_cmd::read_pin(&t.name);
            pin.filter(|p| unshipped.contains(p))
        }
    })
}

/// AC-6: a missing host credential must stop shipping before anything is
/// committed — only relevant once there's a GitHub remote to push to; a
/// project with no remote never touches a credential at all.
fn ensure_host_credential(repo: Option<&str>, token: Option<&str>) -> Result<()> {
    if repo.is_some() && token.is_none() {
        anyhow::bail!(
            "no GitHub credential on the host to push with — run `gh auth login`, then try `moor ship` again. Nothing was committed."
        );
    }
    Ok(())
}

/// `https://github.com/<owner>/<repo>.git` for `repo` ("owner/repo").
fn github_https_url(repo: &str) -> String {
    format!("https://github.com/{repo}.git")
}

/// The commit `moor ship` makes for a spec's work. Executed via
/// `Target::exec` (`docker exec` into the sandbox) by every caller — never
/// a bare host `git commit` — so the commit is authored by whatever git
/// identity is configured inside the container (ADR-0007's synced host
/// identity), not the host's own working directory.
fn commit_argv(subject: &str, body: &str) -> Vec<String> {
    ["git", "commit", "-q", "-m", subject, "-m", body]
        .map(String::from)
        .to_vec()
}

/// The `docker exec` argv that streams `branch`'s commits out of the
/// sandbox as a git bundle to stdout (AC-4) — no bind mount, no `docker
/// cp` (blocked by the read-only rootfs), just a stream through `docker
/// exec`'s own stdout.
fn bundle_argv(container: &str, branch: &str) -> Vec<String> {
    ["exec", container, "git", "bundle", "create", "-", branch]
        .map(String::from)
        .to_vec()
}

/// The host `git` argv that pushes a scratch repo's `FETCH_HEAD` to
/// `branch` on `https_url` (AC-5) — a plain host invocation, never
/// `docker`/`exec`.
fn push_argv(scratch: &str, https_url: &str, branch: &str) -> Vec<String> {
    vec![
        "-C".to_string(),
        scratch.to_string(),
        "push".to_string(),
        https_url.to_string(),
        format!("FETCH_HEAD:refs/heads/{branch}"),
    ]
}

/// Get `branch`'s commits out of the sandbox and onto `https_url` with the
/// host's own credential (ADR-0009): stream a bundle out via `docker exec`
/// (AC-4), fetch it into an ephemeral scratch repo, then push from there
/// with `token` (AC-5). Returns the pushed commit's sha for the audit
/// record (AC-7). The host keeps no standing clone — the scratch
/// directory is removed before returning.
fn push_shipped_branch(t: &Target, branch: &str, https_url: &str, token: &str) -> Result<String> {
    let container = t.m.sandbox_container();
    let dir = std::env::temp_dir().join(format!("moor-ship-{}-{}", t.name, std::process::id()));
    std::fs::create_dir_all(&dir).context("creating a scratch directory for the push")?;
    let cleanup = || {
        let _ = std::fs::remove_dir_all(&dir);
    };

    let bundle_path = dir.join("ship.bundle");
    let argv = bundle_argv(&container, branch);
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    let status = proc::run_stdin_to_file("docker", &args, Path::new("/dev/null"), &bundle_path);
    let status =
        status.and_then(|s| proc::require_success("streaming the branch out of the sandbox", s));
    if let Err(e) = status {
        cleanup();
        return Err(e);
    }

    let scratch = dir.join("scratch");
    let scratch_str = scratch.to_string_lossy().to_string();
    let bundle_str = bundle_path.to_string_lossy().to_string();
    let result: Result<String> = (|| {
        proc::require_success(
            "git init (scratch)",
            proc::run_capture("git", &["init", "-q", &scratch_str])?.0,
        )?;
        proc::require_success(
            "git fetch (bundle -> scratch)",
            proc::run_capture_combined(
                "git",
                &["-C", &scratch_str, "fetch", "-q", &bundle_str, branch],
            )?
            .0,
        )?;
        let (_, sha) = proc::run_capture("git", &["-C", &scratch_str, "rev-parse", "FETCH_HEAD"])?;
        let sha = sha.trim().to_string();

        let argv = push_argv(&scratch_str, https_url, branch);
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let (ok, out) = super::run_git_as_host(token, &args)?;
        if !ok {
            anyhow::bail!("git push failed:\n{out}");
        }
        Ok(sha)
    })();
    cleanup();
    result
}

pub fn run(explicit: Option<String>, named: Option<String>, yes: bool) -> Result<()> {
    let t = Target::resolve(explicit)?;
    let flag = t
        .flag
        .as_ref()
        .map(|p| format!(" --project {p}"))
        .unwrap_or_default();
    let report = t.report()?;
    let Some(slug) = pick(&t, &report, named.as_deref())? else {
        println!("Nothing to ship: no spec has an approved merge waiting.\n");
        t.print_guidance(&report);
        return Ok(());
    };
    manifest::validate_name(&slug).context("spec name")?;
    let Some(spec) = report.specs.iter().find(|s| s.slug == slug) else {
        anyhow::bail!("no spec '{slug}' in this project");
    };
    if !spec.complete {
        anyhow::bail!(
            "{slug} isn't approved for merging yet: it's at {}.\n\n  Next:  moor next{flag}",
            guide::step_label(spec)
        );
    }
    if !yes && !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "moor ship asks before it publishes, and there's no terminal to ask at.\n\n  Next:  moor ship{flag} --yes"
        );
    }

    // What belongs to this spec's change.
    let (_, porcelain) = t.exec(
        "ship",
        &["git", "status", "--porcelain", "--untracked-files=all"],
    )?;
    let (_, runs_json) = t.exec("ship", &["keel", "runs", "--json"])?;
    let runs: serde_json::Value = serde_json::from_str(runs_json.trim()).unwrap_or_default();
    let run_owner: HashMap<String, String> = runs["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some((
                r["id"].as_str()?.to_string(),
                r["spec"].as_str()?.to_string(),
            ))
        })
        .collect();
    let paths = select(&changed_paths(&porcelain), &slug, &run_owner);
    if paths.is_empty() {
        println!("Nothing to ship for {slug}: its work is already committed.\n");
        t.print_guidance(&report);
        return Ok(());
    }

    let (_, head) = t.exec("ship", &["git", "rev-parse", "--abbrev-ref", "HEAD"])?;
    let trunk = plain_branch(&head).context("the sandbox's repository isn't on a branch")?;
    let branch = format!("moor/{slug}");
    let (_, remote) = t.exec("ship", &["git", "remote", "get-url", "origin"])?;
    let repo = github_repo(&remote);
    let (_, spec_md) = t.exec("ship", &["cat", &format!(".keel/specs/{slug}/spec.md")])?;
    let (title, id) = title_and_id(&spec_md, &slug);
    let run_id = super::flow_cmd::newest_run(&runs, &slug);

    // AC-6: a missing host credential must stop shipping before anything
    // is committed — checked here, before the branch is even created.
    let token = repo.as_ref().and_then(|_| super::host_github_token());
    ensure_host_credential(repo.as_deref(), token.as_deref())?;

    println!("{slug} will be committed on branch {branch}:\n");
    for line in listing(&paths) {
        println!("  {line}");
    }
    println!();
    let publish = match &repo {
        Some(r) => format!(", pushed, and opened as a pull request on {r}"),
        None => " (no GitHub remote, so it stays in the sandbox)".to_string(),
    };
    if !yes && !confirm(&format!("Ship {slug}: commit on {branch}{publish}?")) {
        println!("\nNot shipped; nothing was changed.");
        return Ok(());
    }

    let (created, out) = t.exec("ship", &["git", "switch", "-c", &branch])?;
    if !created {
        anyhow::bail!("couldn't create branch {branch}:\n{out}");
    }
    let mut add = vec!["git", "add", "--"];
    add.extend(paths.iter().map(String::as_str));
    let subject = match &id {
        Some(id) => format!("{title} ({id})"),
        None => title.clone(),
    };
    let body = match &run_id {
        Some(r) => format!("Built and checked in a moor sandbox (run {r}); merge approved."),
        None => "Built and checked in a moor sandbox; merge approved.".to_string(),
    };
    let committed = t.exec("ship", &add)?.0 && {
        let commit: Vec<String> = commit_argv(&subject, &body);
        let commit: Vec<&str> = commit.iter().map(String::as_str).collect();
        t.exec("ship", &commit)?.0
    };
    if !committed {
        t.exec("ship", &["git", "switch", &trunk])?;
        anyhow::bail!("couldn't commit {slug}'s work; nothing was pushed");
    }
    println!("==> committed on {branch}");

    let mut pr_url = None;
    if let (Some(repo), Some(token)) = (&repo, &token) {
        let https = github_https_url(repo);
        match push_shipped_branch(&t, &branch, &https, token) {
            Ok(sha) => {
                audit::log_exec_with(
                    &t.name,
                    &t.m,
                    "push",
                    &["git".into(), "push".into(), branch.clone()],
                    Some(0),
                    super::run_cmd::push_data(&branch, Some(&sha)),
                )?;
                let pr_body = format!(
                    "{body}\n\nShipped with `moor ship`. The spec and its evidence are in `.keel/specs/{slug}/` and `.keel/runs/`."
                );
                let (status, out) = proc::run_capture(
                    "gh",
                    &[
                        "pr", "create", "--repo", repo, "--base", &trunk, "--head", &branch,
                        "--title", &subject, "--body", &pr_body,
                    ],
                )?;
                if status.success() {
                    pr_url = Some(out.trim().to_string());
                } else {
                    println!("note: pushed, but opening the pull request failed; open it for {branch} on {repo}");
                }
            }
            Err(e) => println!("note: committed on {branch}, but the push failed: {e:#}"),
        }
    }

    t.exec("ship", &["git", "switch", &trunk])?;
    println!();
    match pr_url {
        Some(url) => {
            println!("Shipped {slug}: {url}");
            println!(
                "The sandbox is back on {trunk}. Once the pull request is merged, bring it up to date:\n\n  Then:  moor run {} -- git pull --ff-only",
                t.name
            );
        }
        None if repo.is_some() => println!(
            "{slug} is committed on {branch} in the sandbox; see the note above. The sandbox is back on {trunk}.\n"
        ),
        None => println!(
            "{slug} is committed on {branch} in the sandbox (no GitHub remote to push to). The sandbox is back on {trunk}.\n"
        ),
    }
    println!("  Next:  moor next{flag}");
    Ok(())
}

/// The files to commit, as the operator should read them: the change's own
/// files one by one, and the pipeline's records for the spec as one line.
fn listing(paths: &[String]) -> Vec<String> {
    let printable = |p: &str| p.chars().filter(|c| !c.is_control()).collect::<String>();
    let (keel, change): (Vec<&String>, Vec<&String>) =
        paths.iter().partition(|p| p.starts_with(".keel/"));
    let mut out: Vec<String> = change.iter().map(|p| printable(p)).collect();
    if !keel.is_empty() {
        let mut runs: Vec<&str> = keel
            .iter()
            .filter_map(|p| p.strip_prefix(".keel/runs/")?.split('/').next())
            .collect();
        runs.dedup();
        out.push(format!(
            "+ its spec and the evidence of {} run(s): {} file(s) under .keel/",
            runs.len(),
            keel.len()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_paths_include_renames_and_untracked() {
        let out = " M src/a.rs\n?? .keel/specs/greet/spec.md\nR  old.rs -> new.rs\n";
        assert_eq!(
            changed_paths(out),
            vec!["src/a.rs", ".keel/specs/greet/spec.md", "new.rs"]
        );
    }

    #[test]
    fn other_specs_and_their_runs_are_left_behind() {
        let paths: Vec<String> = [
            "src/next.rs",
            ".keel/lesson-usage.json",
            ".keel/specs/greet/spec.md",
            ".keel/specs/other/spec.md",
            ".keel/runs/2026-09-29-001/run.json",
            ".keel/runs/2026-09-29-002/run.json",
        ]
        .map(String::from)
        .to_vec();
        let owners: HashMap<String, String> = [
            ("2026-09-29-001".to_string(), "greet".to_string()),
            ("2026-09-29-002".to_string(), "other".to_string()),
        ]
        .into();
        assert_eq!(
            select(&paths, "greet", &owners),
            vec![
                "src/next.rs",
                ".keel/lesson-usage.json",
                ".keel/specs/greet/spec.md",
                ".keel/runs/2026-09-29-001/run.json",
            ]
        );
    }

    #[test]
    fn the_listing_names_code_files_and_summarises_records() {
        let paths: Vec<String> = [
            "src/greet.sh",
            ".keel/specs/greet/spec.md",
            ".keel/runs/2026-09-29-000/run.json",
            ".keel/runs/2026-09-29-000/gates/G2.json",
            ".keel/runs/2026-09-29-001/run.json",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            listing(&paths),
            vec![
                "src/greet.sh",
                "+ its spec and the evidence of 2 run(s): 4 file(s) under .keel/",
            ]
        );
    }

    #[test]
    fn github_remotes_parse_and_odd_ones_do_not() {
        assert_eq!(
            github_repo("https://github.com/daneb/keel.git").as_deref(),
            Some("daneb/keel")
        );
        assert_eq!(
            github_repo("git@github.com:daneb/keel.git\n").as_deref(),
            Some("daneb/keel")
        );
        assert_eq!(github_repo("https://gitlab.com/a/b.git"), None);
        assert_eq!(github_repo("https://github.com/a/b;rm -rf ~"), None);
    }

    #[test]
    fn branch_names_must_be_plain() {
        assert_eq!(plain_branch("master\n").as_deref(), Some("master"));
        assert_eq!(plain_branch("HEAD"), None);
        assert_eq!(plain_branch("main; echo x"), None);
    }

    #[test]
    fn title_comes_from_the_heading_without_control_characters() {
        let md =
            "---\nid: SPEC-0016\nschema: keel.spec/1\nslug: s\n---\n\n# Greet \u{1b}[31mby name\n";
        let (title, id) = title_and_id(md, "s");
        assert_eq!(title, "Greet [31mby name");
        assert_eq!(id.as_deref(), Some("SPEC-0016"));
        assert_eq!(title_and_id("no front matter", "s").0, "s");
    }

    #[test]
    fn absent_host_credential_fails_before_commit() {
        let err = ensure_host_credential(Some("daneb/keel"), None).unwrap_err();
        assert!(err.to_string().contains("gh auth login"));
        assert!(ensure_host_credential(Some("daneb/keel"), Some("tok")).is_ok());
        // No remote at all: never needs a credential.
        assert!(ensure_host_credential(None, None).is_ok());
    }

    #[test]
    fn commit_runs_in_sandbox() {
        // The commit's argv is plain `git commit`, meant to be run via
        // `Target::exec` (docker exec into the sandbox) — never a bare
        // host git invocation.
        let argv = commit_argv("Greet (SPEC-1)", "body text");
        assert_eq!(argv[0], "git");
        assert_eq!(argv[1], "commit");
        assert!(!argv.iter().any(|a| a == "docker"));
    }

    #[test]
    fn commits_leave_via_bundle_stream() {
        let argv = bundle_argv("sample-app-sandbox", "moor/greet");
        assert_eq!(
            argv,
            [
                "exec",
                "sample-app-sandbox",
                "git",
                "bundle",
                "create",
                "-",
                "moor/greet"
            ]
            .map(String::from)
            .to_vec()
        );
        assert!(!argv.iter().any(|a| a == "cp"));
    }

    #[test]
    fn push_runs_on_host() {
        let argv = push_argv(
            "/tmp/moor-ship-scratch",
            "https://github.com/daneb/keel.git",
            "moor/greet",
        );
        assert_eq!(argv[0], "-C");
        assert_eq!(argv[2], "push");
        assert!(argv.iter().any(|a| a.contains("github.com")));
        assert!(!argv.iter().any(|a| a == "docker" || a == "exec"));
    }

    #[test]
    fn host_push_is_recorded_with_ref_and_commit() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let data = super::super::run_cmd::push_data("moor/greet", Some(sha));
        assert_eq!(data["ref"], "moor/greet");
        assert_eq!(data["commit"], sha);
    }
}
