//! `moor approve`, `moor reject` and `moor go`: the commands that move the
//! active spec on, so the operator never types a spec name or a stage.
//! Each one acts on whatever `moor next` would show, and ends by showing
//! `moor next`'s guidance for where that left things.

use super::next_cmd::Target;
use crate::guide;
use anyhow::Result;
use std::io::{BufRead, IsTerminal, Write};

/// Runs `keel <args>` in the sandbox, inheriting its output and logged
/// like any `moor run`.
fn exec(t: &Target, args: &[&str]) -> Result<()> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    super::keel_cmd::run(&t.name, &args)
}

/// What an approval is of, as a sentence fragment.
fn subject(decision: &str, slug: &str) -> String {
    match decision {
        "merge" => format!("merging {slug}"),
        other => format!("the {other} for {slug}"),
    }
}

/// Only an explicit y/yes is a yes. A read error is a no.
fn confirm(reader: &mut impl BufRead, writer: &mut impl Write, question: &str) -> bool {
    let _ = write!(writer, "{question} [y/N] ");
    let _ = writer.flush();
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// The active spec and the approval it's waiting on, or `None` after
/// printing why there's nothing to decide and what to do instead.
fn pending<'a>(
    t: &Target,
    report: &'a guide::NextReport,
) -> Option<(&'a guide::NextSpec, &'static str)> {
    let active = t.active(report);
    let found = active
        .as_ref()
        .and_then(|a| guide::pending_decision(a.spec).map(|d| (a.spec, d)));
    if found.is_none() {
        if let Some(a) = &active {
            println!(
                "Nothing is waiting for your decision: {} is at {}.\n",
                a.spec.slug,
                guide::step_label(a.spec)
            );
        }
        t.print_guidance(report);
    }
    found
}

/// `moor approve`: show what's being approved, ask, then record it.
/// `yes` skips the question, for when there's no terminal to ask at.
pub fn approve(explicit: Option<String>, yes: bool) -> Result<()> {
    let t = Target::resolve(explicit)?;
    let report = t.report()?;
    let Some((spec, decision)) = pending(&t, &report) else {
        return Ok(());
    };
    if !yes && !std::io::stdin().is_terminal() {
        let flag = t
            .flag
            .as_ref()
            .map(|p| format!(" --project {p}"))
            .unwrap_or_default();
        anyhow::bail!(
            "moor approve asks before recording, and there's no terminal to ask at.\n\n  Next:  moor approve{flag} --yes    (approves {} without asking)",
            subject(decision, &spec.slug)
        );
    }

    match guide::artifact_to_read(spec) {
        Some("report") => merge_review(&t, &spec.slug)?,
        Some(artifact) => super::view::run(&t.name, &spec.slug, artifact)?,
        None => {}
    }
    println!();
    let question = format!("Approve {}?", subject(decision, &spec.slug));
    if !yes
        && !confirm(
            &mut std::io::stdin().lock(),
            &mut std::io::stdout(),
            &question,
        )
    {
        println!("\nNot approved; nothing was recorded.\n");
        t.print_guidance(&report);
        return Ok(());
    }

    exec(&t, &["approve", &spec.slug, "--stage", decision])?;
    println!();
    t.print_guidance(&t.report()?);
    Ok(())
}

/// What a merge approval is approving: the change itself and the checks
/// the newest run passed, rather than every run's history. Everything shown
/// comes from the sandbox, so control characters are dropped first.
fn merge_review(t: &Target, slug: &str) -> Result<()> {
    let printable = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
    println!("What {slug}'s build changes (the pipeline's own records aside):\n");
    let outside_keel = [".", ":(exclude).keel"];
    let mut stat = vec!["git", "--no-pager", "diff", "--stat", "HEAD", "--"];
    stat.extend(outside_keel);
    let (_, out) = t.exec("approve", &stat)?;
    for line in out.lines() {
        println!("  {}", printable(line));
    }
    let mut new_files = vec!["git", "ls-files", "--others", "--exclude-standard", "--"];
    new_files.extend(outside_keel);
    let (_, out) = t.exec("approve", &new_files)?;
    for line in out.lines().filter(|l| !l.trim().is_empty()) {
        println!("  new file: {}", printable(line));
    }

    let (_, runs) = t.exec("approve", &["keel", "runs", "--json"])?;
    let runs: serde_json::Value = serde_json::from_str(runs.trim()).unwrap_or_default();
    if let Some(run) = newest_run(&runs, slug) {
        println!("\nChecks on run {run}:");
        for gate in ["G2", "G2.5"] {
            let path = format!(".keel/runs/{run}/gates/{gate}.json");
            let (ok, out) = t.exec("approve", &["cat", &path])?;
            if ok {
                println!("  {}", gate_summary(gate, &out));
            }
        }
    }
    println!("\n  Full diff:  moor view {slug} diff");
    Ok(())
}

/// "G2 pass — 13 passed, 0 failed, 0 blocked", from a gate's JSON result.
fn gate_summary(gate: &str, gate_json: &str) -> String {
    let g: serde_json::Value = serde_json::from_str(gate_json.trim()).unwrap_or_default();
    let count = |v: &str| {
        g["checks"]
            .as_array()
            .map(|c| c.iter().filter(|c| c["verdict"] == v).count())
            .unwrap_or(0)
    };
    let verdict = g["verdict"]
        .as_str()
        .filter(|v| matches!(*v, "pass" | "fail" | "blocked"))
        .unwrap_or("unknown");
    format!(
        "{gate} {verdict} — {} passed, {} failed, {} blocked",
        count("pass"),
        count("fail"),
        count("blocked")
    )
}

/// `moor reject "why"`: record a rejection of whatever is waiting on the
/// operator, with the reason.
///
/// At a normal approval gate (spec/plan/merge) this rejects that gate. At
/// the build step there is no pending *approval* — but the operator can
/// still want to pull the spec back ("don't build this after all"). In
/// that case reject the plan approval that launched the build, with
/// `--force` since the spec has moved past it, so the spec leaves the
/// build step instead of being stuck with no guided way out.
pub fn reject(explicit: Option<String>, why: &str) -> Result<()> {
    if why.trim().is_empty() {
        anyhow::bail!("say why, so the next attempt can address it: moor reject \"why\"");
    }
    let t = Target::resolve(explicit)?;
    let report = t.report()?;

    // A pending gate (spec/plan/merge) is rejected directly.
    if let Some(active) = t.active(&report) {
        if let Some(decision) = guide::pending_decision(active.spec) {
            exec(
                &t,
                &[
                    "approve", &active.spec.slug, "--stage", decision, "--reject", "--note", why,
                ],
            )?;
            println!();
            t.print_guidance(&t.report()?);
            return Ok(());
        }
        // At the build step there is no pending approval, but the plan
        // approval that launched it can be withdrawn, taking the spec out
        // of the build step. `--force` because the spec has progressed
        // past (and may have locked at) that stage.
        if active.spec.stage == "run" {
            println!(
                "{} is building, not at an approval gate — rejecting its approved plan to pull it back.\n",
                active.spec.slug
            );
            exec(
                &t,
                &[
                    "approve",
                    &active.spec.slug,
                    "--stage",
                    "plan",
                    "--reject",
                    "--force",
                    "--note",
                    why,
                ],
            )?;
            println!();
            t.print_guidance(&t.report()?);
            return Ok(());
        }
        // Nothing to reject at this stage: say so, and show the way on.
        println!(
            "Nothing is waiting for your decision: {} is at {}.\n",
            active.spec.slug,
            guide::step_label(active.spec)
        );
        t.print_guidance(&report);
    }
    Ok(())
}

/// `moor go`: take the active spec's automatic steps (checks, planning,
/// the build run) one after another, stopping at the first thing that
/// needs the operator, a failure, or a step that didn't move it on.
/// `check`: take the build step without the agent, re-checking the work
/// already in the sandbox (after fixing it by hand or with `moor ask`).
pub fn go(explicit: Option<String>, check: bool) -> Result<()> {
    let t = Target::resolve(explicit)?;
    let mut report = t.report()?;
    let Some(slug) = t.active(&report).map(|a| a.spec.slug.clone()) else {
        t.print_guidance(&report);
        return Ok(());
    };

    let mut ran = 0;
    let mut last_stage: Option<String> = None;
    while let Some(spec) = report.specs.iter().find(|s| s.slug == slug) {
        if !guide::is_automatic(spec) {
            break;
        }
        if last_stage.as_deref() == Some(spec.stage.as_str()) {
            println!("\n{slug} didn't move on from {}.", guide::step_label(spec));
            break;
        }
        // A rejected or stale approval is re-checked once, then left for the
        // operator: keel keeps reporting the rejection until someone
        // approves, so looping would re-check the same thing forever.
        let rechecking = guide::recheck(spec).is_some();
        let command = guide::recheck(spec).unwrap_or(&spec.command).to_string();
        let Some(("keel", args)) = command.split_once(' ') else {
            anyhow::bail!("don't know how to run `{command}`");
        };
        let mut args: Vec<String> = args.split_whitespace().map(String::from).collect();
        let building = args.first().map(String::as_str) == Some("run");
        if building && check {
            args.push("--no-driver".to_string());
        }
        println!("==> {slug} · {}", guide::step_label(spec));
        last_stage = Some(spec.stage.clone());
        ran += 1;

        let result = super::keel_cmd::run(&t.name, &args);
        report = t.report()?;
        if let Err(e) = result {
            // A build that passed still makes keel exit non-zero: its last
            // check is the merge approval, which is still open. Whether the
            // build passed is its G2 verdict, which is keel's own rule for a
            // passing run.
            if building && build_passed(&t, &slug)? {
                if rechecking {
                    println!("\n{RECHECKED}");
                    break;
                }
                continue;
            }
            println!();
            if building {
                anyhow::bail!("{}", guide::build_failed(&slug, t.flag.as_deref()));
            }
            t.print_guidance(&report);
            return Err(e);
        }
        if rechecking {
            println!("\n{RECHECKED}");
            break;
        }
    }

    if ran == 0 {
        println!("Nothing to run: the next step needs you.\n");
    } else {
        println!();
    }
    t.print_guidance(&report);
    Ok(())
}

const RECHECKED: &str =
    "It passed its checks again. The earlier decision stays on record until you approve the revision.";

/// Whether `slug`'s newest build passed its G2 checks. Reads two things
/// from the sandbox, the run list and that run's G2 result, and uses only
/// the verdict: nothing from either is printed or put into a command.
fn build_passed(t: &Target, slug: &str) -> Result<bool> {
    let container = t.m.sandbox_container();
    let (status, out) =
        crate::proc::run_capture("docker", &["exec", &container, "keel", "runs", "--json"])?;
    if !status.success() {
        return Ok(false);
    }
    let runs: serde_json::Value = serde_json::from_str(out.trim()).unwrap_or_default();
    let Some(id) = newest_run(&runs, slug) else {
        return Ok(false);
    };
    let path = format!(".keel/runs/{id}/gates/G2.json");
    let (status, out) = crate::proc::run_capture("docker", &["exec", &container, "cat", &path])?;
    Ok(status.success() && gate_passed(&out))
}

/// The id of `slug`'s newest run, if it has the shape of a run id.
pub(crate) fn newest_run(runs: &serde_json::Value, slug: &str) -> Option<String> {
    runs["runs"]
        .as_array()?
        .iter()
        .filter(|r| r["spec"] == slug)
        .filter_map(|r| r["id"].as_str())
        .next_back()
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
        .map(str::to_string)
}

fn gate_passed(gate_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(gate_json.trim())
        .is_ok_and(|g| g["verdict"] == "pass")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_explicit_yes_confirms() {
        for (input, expected) in [
            ("y\n", true),
            ("YES\n", true),
            ("n\n", false),
            ("\n", false),
            ("sure\n", false),
        ] {
            let mut out = Vec::new();
            let got = confirm(&mut std::io::Cursor::new(input), &mut out, "Approve?");
            assert_eq!(got, expected, "{input:?}");
            assert!(String::from_utf8(out).unwrap().contains("Approve? [y/N]"));
        }
    }

    #[test]
    fn the_newest_run_for_the_spec_is_found() {
        let runs = serde_json::json!({"runs": [
            {"id": "2026-09-29-000", "spec": "greet"},
            {"id": "2026-09-29-001", "spec": "other"},
            {"id": "2026-09-29-002", "spec": "greet"},
            {"id": "../../etc", "spec": "evil"},
        ]});
        assert_eq!(
            newest_run(&runs, "greet").as_deref(),
            Some("2026-09-29-002")
        );
        assert_eq!(
            newest_run(&runs, "evil"),
            None,
            "a path-shaped id is refused"
        );
        assert_eq!(newest_run(&runs, "missing"), None);
    }

    #[test]
    fn only_a_pass_verdict_counts() {
        assert!(gate_passed(r#"{"gate":"G2","verdict":"pass"}"#));
        assert!(!gate_passed(r#"{"gate":"G2","verdict":"fail"}"#));
        assert!(!gate_passed(r#"{"gate":"G2","verdict":"blocked"}"#));
        assert!(!gate_passed("not json"));
    }

    #[test]
    fn gate_summary_counts_checks() {
        let json = r#"{"gate":"G2","verdict":"pass","checks":[{"verdict":"pass"},{"verdict":"pass"},{"verdict":"blocked"}]}"#;
        assert_eq!(
            gate_summary("G2", json),
            "G2 pass — 2 passed, 0 failed, 1 blocked"
        );
        assert_eq!(
            gate_summary("G2", "junk"),
            "G2 unknown — 0 passed, 0 failed, 0 blocked"
        );
    }

    #[test]
    fn subject_reads_naturally() {
        assert_eq!(subject("plan", "login"), "the plan for login");
        assert_eq!(subject("merge", "login"), "merging login");
    }
}
