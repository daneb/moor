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
        Some("report") => exec(&t, &["report", &spec.slug])?,
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

/// `moor reject "why"`: record a rejection of whatever is waiting on the
/// operator, with the reason.
pub fn reject(explicit: Option<String>, why: &str) -> Result<()> {
    if why.trim().is_empty() {
        anyhow::bail!("say why, so the next attempt can address it: moor reject \"why\"");
    }
    let t = Target::resolve(explicit)?;
    let report = t.report()?;
    let Some((spec, decision)) = pending(&t, &report) else {
        return Ok(());
    };
    exec(
        &t,
        &[
            "approve", &spec.slug, "--stage", decision, "--reject", "--note", why,
        ],
    )?;
    println!();
    t.print_guidance(&t.report()?);
    Ok(())
}

/// `moor go`: take the active spec's automatic steps (checks, planning,
/// the build run) one after another, stopping at the first thing that
/// needs the operator, a failure, or a step that didn't move it on.
pub fn go(explicit: Option<String>) -> Result<()> {
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
        let Some(("keel", args)) = spec.command.split_once(' ') else {
            anyhow::bail!("don't know how to run `{}`", spec.command);
        };
        let args: Vec<String> = args.split_whitespace().map(String::from).collect();
        println!("==> {slug} · {}", guide::step_label(spec));
        last_stage = Some(spec.stage.clone());
        ran += 1;

        let result = super::keel_cmd::run(&t.name, &args);
        report = t.report()?;
        if let Err(e) = result {
            println!();
            t.print_guidance(&report);
            return Err(e);
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
    fn subject_reads_naturally() {
        assert_eq!(subject("plan", "login"), "the plan for login");
        assert_eq!(subject("merge", "login"), "merging login");
    }
}
