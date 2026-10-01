//! `moor ship`: take a spec whose merge you approved and get it to GitHub.
//! Commits the spec's work on its own branch in the sandbox, pushes it from
//! the sandbox, opens a pull request from the host, and returns the sandbox
//! to its trunk so the next spec starts from a clean tree.
//!
//! Approval stays a separate act (`moor approve`); shipping publishes, so
//! it shows exactly what it will commit and asks first.

use super::next_cmd::Target;
use crate::{guide, manifest, proc};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Write};

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
    let committed = t.exec("ship", &add)?.0
        && t.exec(
            "ship",
            &["git", "commit", "-q", "-m", &subject, "-m", &body],
        )?
        .0;
    if !committed {
        t.exec("ship", &["git", "switch", &trunk])?;
        anyhow::bail!("couldn't commit {slug}'s work; nothing was pushed");
    }
    println!("==> committed on {branch}");

    let mut pr_url = None;
    if let Some(repo) = &repo {
        // The same credential helper `moor new --github` uses: the token is
        // read inside the sandbox, never passed through any argv.
        let helper =
            "credential.helper=!f() { echo username=x-access-token; echo \"password=$GITHUB_TOKEN\"; }; f";
        let pushed = super::run_cmd::run(
            &t.name,
            &["git", "-c", helper, "push", "-u", "origin", &branch].map(String::from),
        );
        match pushed {
            Ok(()) => {
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
}
