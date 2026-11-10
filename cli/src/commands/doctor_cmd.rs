//! `moor doctor`: read-only visibility into the git state of a project's
//! sandbox workspace, before you build on top of it.
//!
//! The sandbox is isolated, so you otherwise cannot see which branch the
//! repo is on, whether it has uncommitted work, or whether it is behind
//! the remote — all of which matter before starting a feature. This
//! reports that state. It changes no branch, HEAD, or working-tree file
//! (no checkout, no merge): it is purely a lens. The one exception is a
//! best-effort host-side fetch that refreshes trunk's local
//! remote-tracking ref before reporting (AC-7, see sync_cmd::TrunkRef),
//! so the behind count below is truthful rather than only as fresh as
//! the last fetch. The hard gate that will *refuse* to build on a bad
//! state is a separate, later change built on top of this.

use crate::{manifest::Manifest, paths, proc};
use anyhow::Result;

/// A sandbox repo's git state, parsed from `git status --porcelain=v2
/// --branch`. All fields are best-effort: a field that git did not report
/// (e.g. no upstream) is left `None`, never guessed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GitState {
    /// The branch name, or None when HEAD is detached.
    pub branch: Option<String>,
    /// True when `git status` reports a detached HEAD.
    pub detached: bool,
    /// The configured upstream (e.g. "origin/master"), if any.
    pub upstream: Option<String>,
    /// Commits ahead of / behind the upstream, if an upstream is set.
    pub ahead: u32,
    pub behind: u32,
    /// Number of changed (tracked-modified, staged, or untracked) entries.
    pub dirty: u32,
}

impl GitState {
    pub fn is_clean(&self) -> bool {
        self.dirty == 0
    }
}

/// Parse `git status --porcelain=v2 --branch` output. The `# branch.*`
/// header lines carry oid/head/upstream/ab; every other non-empty line is
/// a changed entry (`1`/`2`/`u`/`?`). Pure, so it is unit-tested without a
/// container.
pub fn parse_porcelain_v2(out: &str) -> GitState {
    let mut st = GitState::default();
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            let head = rest.trim();
            if head == "(detached)" {
                st.detached = true;
            } else {
                st.branch = Some(head.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            st.upstream = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            // Format: "+<ahead> -<behind>"
            for tok in rest.split_whitespace() {
                if let Some(n) = tok.strip_prefix('+') {
                    st.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = tok.strip_prefix('-') {
                    st.behind = n.parse().unwrap_or(0);
                }
            }
        } else if line.starts_with('#') || line.trim().is_empty() {
            // other header, or blank — not a changed entry
        } else {
            // 1/2/u/? entry lines each count as one changed path.
            st.dirty += 1;
        }
    }
    st
}

/// A one-line human summary of the state, with the trunk it is compared
/// against (so "on master" vs "NOT on master" reads correctly).
pub fn summary(st: &GitState, trunk: &str) -> String {
    let mut parts = Vec::new();
    if st.detached {
        parts.push("detached HEAD (not on a branch)".to_string());
    } else if let Some(b) = &st.branch {
        if b == trunk {
            parts.push(format!("on {b}"));
        } else {
            parts.push(format!("on {b} (NOT {trunk})"));
        }
    } else {
        parts.push("branch unknown".to_string());
    }
    parts.push(if st.is_clean() {
        "clean".to_string()
    } else {
        format!("{} uncommitted change(s)", st.dirty)
    });
    match &st.upstream {
        Some(up) if st.ahead > 0 || st.behind > 0 => {
            let mut rel = Vec::new();
            if st.ahead > 0 {
                rel.push(format!("{} ahead", st.ahead));
            }
            if st.behind > 0 {
                rel.push(format!("{} behind", st.behind));
            }
            parts.push(format!("{} vs {up}", rel.join(", ")));
        }
        Some(up) => parts.push(format!("up to date with {up}")),
        None => parts.push("no upstream tracked".to_string()),
    }
    parts.join(" · ")
}

/// Best-effort: advance the sandbox's local trunk *tracking* ref
/// (`refs/remotes/origin/<trunk>`) from origin via a host fetch (AC-7),
/// so the behind count reported below reflects origin's current tip
/// rather than only the last fetch. Never touches HEAD, the index, any
/// branch ref, or the working tree (`sync_cmd::TrunkRef::Tracking`), and
/// never fails doctor's own report: a non-GitHub origin or a missing
/// host credential just leaves the count as fresh as the last fetch,
/// same as before this existed.
fn refresh_trunk_tracking(name: &str, m: &Manifest) {
    let _ = crate::commands::sync_cmd::fetch_trunk(
        name,
        m,
        crate::commands::sync_cmd::TrunkRef::Tracking,
    );
}

/// The "behind trunk" advice line (AC-8): names `moor sync`, the ref
/// update that never disturbs an active build, not `moor pull`, which
/// rewrites whatever branch is currently checked out.
fn behind_advice(name: &str, behind: u32, upstream: Option<&str>) -> String {
    format!(
        "  ! {behind} commit(s) behind {} — refresh first:  moor sync -p {name}",
        upstream.unwrap_or("the remote")
    )
}

/// The sandbox's git state, or None if the workspace isn't a git repo.
fn git_state(m: &Manifest) -> Result<Option<GitState>> {
    let container = m.sandbox_container();
    // Is it a repo at all?
    let (st, _) = proc::run_capture(
        "docker",
        &["exec", &container, "git", "rev-parse", "--git-dir"],
    )?;
    if !st.success() {
        return Ok(None);
    }
    let (_, out) = proc::run_capture(
        "docker",
        &[
            "exec",
            &container,
            "git",
            "status",
            "--porcelain=v2",
            "--branch",
        ],
    )?;
    Ok(Some(parse_porcelain_v2(&out)))
}

pub fn run(explicit: Option<String>) -> Result<()> {
    let name = super::resolve_project_announced(explicit)?;
    let m = Manifest::load(&paths::manifest_path(&name)?)?;

    if super::running_projects(std::slice::from_ref(&name))?.is_empty() {
        anyhow::bail!(
            "{name} isn't running, so its git state can't be read.\n\n  Next:  moor up {name}"
        );
    }

    // AC-7: refresh trunk's tracking ref from origin first, best-effort,
    // so the behind count below isn't stale.
    refresh_trunk_tracking(&name, &m);

    // The trunk to compare against: the upstream's branch if tracked, else
    // "master" as the common default. Read-only — doctor never changes it.
    let state = git_state(&m)?;
    match state {
        None => {
            println!("{name}: the workspace is not a git repository yet.");
        }
        Some(st) => {
            let trunk = st
                .upstream
                .as_deref()
                .and_then(|u| u.rsplit('/').next())
                .unwrap_or("master")
                .to_string();
            println!("{name}: {}", summary(&st, &trunk));
            // Spell out the concerns a build would care about, plainly.
            if st.detached {
                println!("  ! HEAD is detached — commits here aren't on any branch.");
            } else if st.branch.as_deref() != Some(trunk.as_str()) {
                println!(
                    "  ! not on {trunk} — a build here commits onto this branch, not {trunk}."
                );
            }
            if !st.is_clean() {
                println!(
                    "  ! {} uncommitted change(s) — a build starts on top of them.",
                    st.dirty
                );
            }
            if st.behind > 0 {
                println!(
                    "{}",
                    behind_advice(&name, st.behind, st.upstream.as_deref())
                );
            }
            if st.is_clean()
                && !st.detached
                && st.branch.as_deref() == Some(trunk.as_str())
                && st.behind == 0
            {
                println!("  ✓ clean, on {trunk}, nothing to refresh — good to build.");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_on_branch_with_upstream() {
        let out = "\
# branch.oid abc123
# branch.head master
# branch.upstream origin/master
# branch.ab +0 -0
";
        let st = parse_porcelain_v2(out);
        assert_eq!(st.branch.as_deref(), Some("master"));
        assert!(!st.detached);
        assert_eq!(st.upstream.as_deref(), Some("origin/master"));
        assert_eq!((st.ahead, st.behind, st.dirty), (0, 0, 0));
        assert!(st.is_clean());
    }

    #[test]
    fn counts_dirty_entries() {
        let out = "\
# branch.head feature-x
# branch.upstream origin/feature-x
# branch.ab +2 -3
1 .M N... 100644 100644 100644 aaa bbb config.yaml
2 R. N... 100644 100644 100644 ccc ddd R100 new.py\told.py
u UU N... ... foo
? untracked.txt
";
        let st = parse_porcelain_v2(out);
        assert_eq!(st.branch.as_deref(), Some("feature-x"));
        assert_eq!((st.ahead, st.behind), (2, 3));
        assert_eq!(st.dirty, 4);
        assert!(!st.is_clean());
    }

    #[test]
    fn detects_detached_head() {
        let out = "# branch.oid abc123\n# branch.head (detached)\n";
        let st = parse_porcelain_v2(out);
        assert!(st.detached);
        assert_eq!(st.branch, None);
    }

    #[test]
    fn no_upstream_leaves_fields_none() {
        let out = "# branch.head master\n";
        let st = parse_porcelain_v2(out);
        assert_eq!(st.upstream, None);
        assert_eq!((st.ahead, st.behind), (0, 0));
    }

    #[test]
    fn summary_flags_off_trunk_and_dirty() {
        let st = GitState {
            branch: Some("feature-x".into()),
            upstream: Some("origin/feature-x".into()),
            ahead: 1,
            behind: 2,
            dirty: 3,
            ..Default::default()
        };
        let s = summary(&st, "master");
        assert!(s.contains("NOT master"), "{s}");
        assert!(s.contains("3 uncommitted"), "{s}");
        assert!(s.contains("1 ahead"), "{s}");
        assert!(s.contains("2 behind"), "{s}");
    }

    #[test]
    fn summary_clean_on_trunk_up_to_date() {
        let st = GitState {
            branch: Some("master".into()),
            upstream: Some("origin/master".into()),
            ..Default::default()
        };
        let s = summary(&st, "master");
        assert!(s.contains("on master"), "{s}");
        assert!(s.contains("clean"), "{s}");
        assert!(s.contains("up to date"), "{s}");
    }

    #[test]
    fn behind_count_reflects_a_host_fetch() {
        // Before refresh_trunk_tracking's host fetch lands origin's new
        // commits into the local tracking ref, the ab line (and so the
        // behind count) reflects only what the last fetch already knew.
        let stale = "# branch.head master\n# branch.upstream origin/master\n# branch.ab +0 -0\n";
        // After it runs, the same `git status` on the same checkout
        // reports the real count against origin's current tip.
        let fresh = "# branch.head master\n# branch.upstream origin/master\n# branch.ab +0 -5\n";
        assert_eq!(parse_porcelain_v2(stale).behind, 0);
        assert_eq!(parse_porcelain_v2(fresh).behind, 5);
    }

    #[test]
    fn behind_advice_names_sync_not_pull() {
        let advice = behind_advice("demo", 3, Some("origin/master"));
        assert!(advice.contains("moor sync"));
        assert!(!advice.contains("moor pull"));
    }
}
