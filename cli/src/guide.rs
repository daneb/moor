//! Where a project's work stands and the one command that moves it on.
//!
//! keel already decides the next step (`keel next --json`), but it answers
//! in keel's own terms: a stage key and a `keel ...` command that only
//! runs inside the sandbox. This module turns that into guidance an
//! operator can act on from the host without translating anything:
//!
//! 1. every command printed runs as-is on the host;
//! 2. one active spec per project, so stale specs don't bury the step;
//! 3. stages are described in plain words, with what to do about them.
//!
//! Pure on purpose — `commands::next_cmd` does the IO — so every rule
//! above is asserted directly.

use serde::Deserialize;

/// Matches `keel next --json`'s "schema": "keel.next/1".
#[derive(Debug, Deserialize, Default)]
pub struct NextReport {
    #[serde(default)]
    pub blockers: Vec<Blocker>,
    #[serde(default)]
    pub specs: Vec<NextSpec>,
}

#[derive(Debug, Deserialize)]
pub struct Blocker {
    pub reason: String,
    pub command: String,
}

#[derive(Debug, Deserialize)]
pub struct NextSpec {
    pub slug: String,
    pub stage: String,
    pub command: String,
    #[serde(default)]
    pub complete: bool,
    /// Present at an approval stage (keel 0.11+).
    #[serde(default)]
    pub approval: Option<Approval>,
}

/// Where a pending approval stands: `absent` (never decided), `rejected`, or
/// `superseded` (the artefact changed after it was approved).
#[derive(Debug, Deserialize)]
pub struct Approval {
    pub stage: String,
    pub standing: String,
    #[serde(default)]
    pub by: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// The command that re-checks the artefact once it has been revised.
    #[serde(default)]
    pub recheck: Option<String>,
}

/// The approval if it was rejected or went stale, i.e. the artefact needs
/// revising or re-checking before it can be approved.
fn revising(spec: &NextSpec) -> Option<&Approval> {
    spec.approval
        .as_ref()
        .filter(|a| a.standing == "rejected" || a.standing == "superseded")
}

/// The command `moor go` runs to re-check a rejected or stale approval's
/// artefact, taken from keel like every other step's command.
pub fn recheck(spec: &NextSpec) -> Option<&str> {
    revising(spec).and_then(|a| a.recheck.as_deref())
}

/// Text that came out of the sandbox, made safe to print: control
/// characters (terminal escape sequences among them) are dropped.
fn printable(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// keel's pipeline has seven steps before `complete` (keel
/// `pipeline::Stage`, whose wire keys are a stable schema).
const TOTAL_STEPS: u8 = 7;

/// What one stage means for the operator.
struct Step {
    n: u8,
    title: &'static str,
    what: &'static str,
    /// The artifact to read before deciding, or to look at when the step's
    /// checks fail: `spec`, `plan`, or `report` (what a run did).
    read: Option<&'static str>,
    /// The approval this step asks the operator for. `None` means the step
    /// is one `moor go` can take on its own.
    decides: Option<&'static str>,
}

fn step(stage: &str) -> Option<Step> {
    let s = |n, title, what, read, decides| Step {
        n,
        title,
        what,
        read,
        decides,
    };
    Some(match stage {
        "spec" => s(
            1,
            "check the spec",
            "The spec hasn't passed its checks yet. Run them; if they fail, fix the spec and run them again.",
            Some("spec"),
            None,
        ),
        "spec_approval" => s(
            2,
            "approve the spec",
            "The spec passed its checks. Read it, then approve or reject it.",
            Some("spec"),
            Some("spec"),
        ),
        "plan" => s(
            3,
            "plan the work",
            "The spec is approved. Next, a plan and its tasks are drafted from it.",
            None,
            None,
        ),
        "plan_gate" => s(
            4,
            "check the plan",
            "A plan exists but hasn't passed its checks. Run them; if they fail, read the plan to see why.",
            Some("plan"),
            None,
        ),
        "plan_approval" => s(
            5,
            "approve the plan",
            "The plan passed its checks. Read it, then approve or reject it.",
            Some("plan"),
            Some("plan"),
        ),
        "run" => s(
            6,
            "build it",
            "The plan is approved. The agent works through the tasks, and the result is checked.",
            None,
            None,
        ),
        "merge_approval" => s(
            7,
            "approve the merge",
            "A run passed. Review what it did, then approve or reject the merge.",
            Some("report"),
            Some("merge"),
        ),
        _ => return None,
    })
}

/// Why this spec is the one being shown.
#[derive(Debug, PartialEq, Eq)]
pub enum Why {
    Pinned,
    OnlyOne,
    MostRecent,
    FurthestAlong,
}

pub struct Active<'a> {
    pub spec: &'a NextSpec,
    pub why: Why,
    /// A pin that no longer applies, named so it isn't silently ignored.
    pub stale_pin: Option<String>,
}

fn stage_rank(stage: &str) -> u8 {
    step(stage).map(|s| s.n).unwrap_or(0)
}

/// The spec guidance should be about. A pin wins while it is still in
/// progress; otherwise the one in-progress spec, else the one the audit
/// chain touched last, else the one furthest along. `chain_lines` is the
/// project's host audit chain, oldest first.
pub fn pick_active<'a>(
    report: &'a NextReport,
    pinned: Option<&str>,
    chain_lines: &[String],
) -> Option<Active<'a>> {
    let open: Vec<&NextSpec> = report.specs.iter().filter(|s| !s.complete).collect();
    let stale_pin = match pinned {
        Some(p) => match open.iter().find(|s| s.slug == p) {
            Some(spec) => {
                return Some(Active {
                    spec,
                    why: Why::Pinned,
                    stale_pin: None,
                })
            }
            None => Some(p.to_string()),
        },
        None => None,
    };
    let active = |spec, why| {
        Some(Active {
            spec,
            why,
            stale_pin: stale_pin.clone(),
        })
    };
    match open.as_slice() {
        [] => None,
        [only] => active(only, Why::OnlyOne),
        _ => {
            // A slug appears quoted in any chain entry that concerns it: an
            // exec's argv, or a folded keel gate/run/approval payload.
            let recent = chain_lines.iter().rev().find_map(|line| {
                open.iter()
                    .find(|s| line.contains(&format!("\"{}\"", s.slug)))
                    .copied()
            });
            match recent {
                Some(spec) => active(spec, Why::MostRecent),
                None => {
                    let furthest = open
                        .iter()
                        .copied()
                        .max_by_key(|s| stage_rank(&s.stage))
                        .expect("open is non-empty");
                    active(furthest, Why::FurthestAlong)
                }
            }
        }
    }
}

/// `--project <name>` when the project was named explicitly, so a printed
/// command resolves to the same project it was printed for.
fn project_flag(explicit: Option<&str>) -> String {
    explicit
        .map(|p| format!(" --project {p}"))
        .unwrap_or_default()
}

/// The approval a spec's current stage is waiting on (`spec`, `plan` or
/// `merge`), or `None` if nothing needs the operator's decision.
pub fn pending_decision(spec: &NextSpec) -> Option<&'static str> {
    step(&spec.stage).and_then(|s| s.decides)
}

/// What to show before an approval: `spec`, `plan`, or `report`.
pub fn artifact_to_read(spec: &NextSpec) -> Option<&'static str> {
    step(&spec.stage).and_then(|s| s.read)
}

/// True for a step `moor go` can take without the operator: a known,
/// unfinished stage that isn't an approval.
pub fn is_automatic(spec: &NextSpec) -> bool {
    !spec.complete
        && (recheck(spec).is_some() || step(&spec.stage).is_some_and(|s| s.decides.is_none()))
}

/// keel's own command, made runnable from the host: `keel <args>` becomes
/// `moor keel <args>`. Taken from keel rather than rebuilt here, so moor
/// follows keel if keel changes what a stage's next command is.
pub fn host_command(keel_cmd: &str, explicit: Option<&str>) -> String {
    let args = keel_cmd.strip_prefix("keel ").unwrap_or(keel_cmd);
    format!("moor keel{} {args}", project_flag(explicit))
}

fn read_command(slug: &str, artifact: &str, explicit: Option<&str>) -> String {
    let flag = project_flag(explicit);
    match artifact {
        "report" => format!("moor keel{flag} report {slug}"),
        _ => format!("moor view{flag} {slug} {artifact}"),
    }
}

/// The step a spec is at, as "step N of 7: title", or "done".
pub fn step_label(spec: &NextSpec) -> String {
    match (spec.complete, step(&spec.stage)) {
        (true, _) => "done".to_string(),
        (false, Some(s)) => {
            let status = match revising(spec).map(|a| a.standing.as_str()) {
                Some("rejected") => " (rejected)",
                Some(_) => " (changed since it was approved)",
                None => "",
            };
            format!("step {} of {TOTAL_STEPS}: {}{status}", s.n, s.title)
        }
        (false, None) => format!("at `{}`", spec.stage),
    }
}

/// The full guidance for one project, as printable lines.
pub fn render(
    project: &str,
    explicit: Option<&str>,
    report: &NextReport,
    active: Option<&Active>,
) -> Vec<String> {
    let mut out = vec![];
    let flag = project_flag(explicit);

    for b in &report.blockers {
        if b.reason == "no specs" {
            continue; // Covered by the "no spec in progress" guidance.
        }
        out.push(format!("Blocked: {}.", b.reason));
        out.push(format!("  Fix:  {}", host_command(&b.command, explicit)));
        out.push(String::new());
    }

    let Some(active) = active else {
        out.push(format!("{project} · no spec in progress"));
        out.push(String::new());
        out.push(
            "  Start one: give it a short name (e.g. login-endpoint), write it on this Mac,"
                .to_string(),
        );
        out.push("  then send it in.".to_string());
        out.push(String::new());
        out.push("  Next:  moor spec new <name>".to_string());
        return out;
    };

    let spec = active.spec;
    out.push(format!("{project} · {} · {}", spec.slug, step_label(spec)));
    if let Some(p) = &active.stale_pin {
        out.push(format!(
            "  (pinned spec `{p}` is done or gone, so showing this one instead)"
        ));
    }
    let others = report
        .specs
        .iter()
        .filter(|s| !s.complete && s.slug != spec.slug)
        .count();
    let pin_hint = format!("pin another with: moor use {project} --spec <name>");
    match active.why {
        Why::MostRecent if others > 0 => {
            out.push(format!("  (the spec you worked on last; {pin_hint})"))
        }
        Why::FurthestAlong if others > 0 => {
            out.push(format!("  (the spec furthest along; {pin_hint})"))
        }
        _ => {}
    }
    out.push(String::new());

    match step(&spec.stage) {
        Some(_) if revising(spec).is_some() => {
            out.extend(revise_lines(spec, revising(spec).unwrap(), &flag));
        }
        Some(s) => {
            out.push(format!("  {}", s.what));
            out.push(String::new());
            if spec.stage == "spec" {
                // Written on the host, a spec is fixed there and sent again;
                // one written in the sandbox can just be checked again.
                out.push(format!(
                    "  Next:  moor spec push{flag} <file>   (after fixing your copy)"
                ));
                out.push(format!(
                    "  Or:    moor go{flag}                 (check it again as it stands)"
                ));
                out.push(format!(
                    "  See it:  {}",
                    read_command(&spec.slug, "spec", explicit)
                ));
            } else if s.decides.is_some() {
                let shown = match s.read {
                    Some("report") => "what the run did".to_string(),
                    Some(artifact) => format!("the {artifact}"),
                    None => "it".to_string(),
                };
                out.push(format!(
                    "  Next:  moor approve{flag}      (shows {shown} first, then asks)"
                ));
                out.push(format!("  Or:    moor reject{flag} \"why\""));
            } else {
                out.push(format!("  Next:  moor go{flag}"));
            }
            if let (Some(artifact), None, false) = (s.read, s.decides, spec.stage == "spec") {
                out.push(format!(
                    "  If it fails:  {}",
                    read_command(&spec.slug, artifact, explicit)
                ));
            }
        }
        None => {
            // A stage this moor doesn't know yet: still point at keel's
            // own next command rather than guessing.
            out.push(format!(
                "  This spec is at `{}`, a step this version of moor doesn't know.",
                spec.stage
            ));
            out.push(String::new());
            out.push(format!(
                "  Next:  {}",
                host_command(&spec.command, explicit)
            ));
        }
    }

    if others > 0 {
        out.push(String::new());
        let noun = if others == 1 { "spec" } else { "specs" };
        out.push(format!(
            "  {others} other {noun} in progress. See all: moor next{flag} --all"
        ));
    }
    out
}

/// What to do about a rejected or stale approval: who rejected it and why,
/// then revise, check again, approve. The note and name are shown but never
/// put into a suggested command, since both come from files the agent can
/// write; the only thing embedded is the slug, and only if it has a slug's
/// shape.
fn revise_lines(spec: &NextSpec, a: &Approval, flag: &str) -> Vec<String> {
    let what = match a.stage.as_str() {
        "merge" => "The build",
        "plan" => "The plan",
        _ => "The spec",
    };
    let mut out = vec![];
    if a.standing == "rejected" {
        let by =
            a.by.as_deref()
                .map(printable)
                .unwrap_or_else(|| "someone".into());
        match a
            .note
            .as_deref()
            .map(printable)
            .filter(|n| !n.trim().is_empty())
        {
            Some(note) => out.push(format!("  {what} was rejected by {by}: \"{note}\"")),
            None => out.push(format!(
                "  {what} was rejected by {by}, with no reason given."
            )),
        }
        out.push("  Revise it, check it again, then approve it.".to_string());
    } else {
        out.push(format!(
            "  {what} changed after it was approved, so check it again, then approve it."
        ));
    }
    out.push(String::new());
    let slug = if crate::manifest::validate_name(&spec.slug).is_ok() {
        spec.slug.as_str()
    } else {
        "<spec>"
    };
    if a.standing == "rejected" {
        let revise = match a.stage.as_str() {
            "spec" => format!("moor spec push{flag} <file>   (after revising your copy)"),
            "plan" => format!(
                "moor ask{flag} --role build \"Revise the plan and tasks for {slug} to address its latest rejection in .keel/specs/{slug}/approvals.jsonl\""
            ),
            _ => format!(
                "moor ask{flag} --role build \"Change {slug}'s work to address its latest merge rejection in .keel/specs/{slug}/approvals.jsonl\""
            ),
        };
        out.push(format!("  Revise:  {revise}"));
        if a.stage == "merge" {
            // The revision is already in the sandbox; rebuilding from the
            // spec would throw it away.
            out.push(format!(
                "  Then:    moor go{flag} --check   (checks it again without rebuilding)"
            ));
        } else {
            out.push(format!("  Then:    moor go{flag}        (checks it again)"));
        }
    } else {
        out.push(format!("  Next:    moor go{flag}        (checks it again)"));
    }
    out.push(format!("  Then:    moor approve{flag}"));
    out
}

/// What to do after a build that didn't pass its checks, which keel has
/// just listed. Retrying with `moor go` alone repeats the whole build from
/// the spec, without the agent ever being told what failed, so the first
/// suggestion is to fix the named failures and re-check without rebuilding.
pub fn build_failed(slug: &str, explicit: Option<&str>) -> String {
    let flag = project_flag(explicit);
    let slug = if crate::manifest::validate_name(slug).is_ok() {
        slug
    } else {
        "<spec>"
    };
    [
        format!("{slug}'s build didn't pass its checks; the output above says why."),
        String::new(),
        format!(
            "  Next:  moor ask{flag} --role build \"Fix what the latest build of {slug} failed on; its failed checks are in its newest run under .keel/runs/\""
        ),
        format!("  Then:  moor go{flag} --check      (checks the fix without rebuilding)"),
        format!("  Or:    moor go{flag}              (the agent builds it again from the spec)"),
        format!("  See:   moor view{flag} {slug} report"),
    ]
    .join("\n")
}

/// Every spec and its step, the active one marked.
pub fn render_all(report: &NextReport, active: Option<&Active>) -> Vec<String> {
    let width = report.specs.iter().map(|s| s.slug.len()).max().unwrap_or(0);
    report
        .specs
        .iter()
        .map(|s| {
            let marker = match active {
                Some(a) if a.spec.slug == s.slug => "▸",
                _ => " ",
            };
            format!("{marker} {:width$}  {}", s.slug, step_label(s))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(slug: &str, stage: &str) -> NextSpec {
        let command = match stage {
            "spec_approval" => format!("keel approve {slug} --stage spec"),
            "plan_gate" => format!("keel gate g1 {slug}"),
            "plan_approval" => format!("keel approve {slug} --stage plan"),
            "run" => format!("keel run {slug}"),
            "complete" => "keel spec new <slug>".to_string(),
            _ => format!("keel {stage} {slug}"),
        };
        NextSpec {
            slug: slug.to_string(),
            stage: stage.to_string(),
            command,
            complete: stage == "complete",
            approval: None,
        }
    }

    fn with_approval(mut s: NextSpec, stage: &str, standing: &str, note: Option<&str>) -> NextSpec {
        let recheck = match stage {
            "spec" => format!("keel gate g0 {}", s.slug),
            "plan" => format!("keel gate g1 {}", s.slug),
            _ => format!("keel run {}", s.slug),
        };
        s.approval = Some(Approval {
            stage: stage.to_string(),
            standing: standing.to_string(),
            by: (standing == "rejected").then(|| "Dane Balia".to_string()),
            note: note.map(String::from),
            recheck: (standing != "absent").then_some(recheck),
        });
        s
    }

    #[test]
    fn a_failed_build_points_at_fixing_then_checking() {
        let text = build_failed("login", Some("p"));
        assert!(text.contains("the output above says why"), "{text}");
        assert!(text.contains("moor ask --project p --role build"), "{text}");
        assert!(text.contains("moor go --project p --check"), "{text}");
        assert!(
            text.contains("moor view --project p login report"),
            "{text}"
        );
        assert!(!text.contains("keel "), "{text}");
    }

    #[test]
    fn parses_the_approval_object() {
        let r: NextReport = serde_json::from_str(
            r#"{"specs":[{"slug":"a","stage":"plan_approval","command":"keel approve a --stage plan","complete":false,
                "approval":{"stage":"plan","standing":"rejected","by":"D","note":null,"recheck":"keel gate g1 a"}}]}"#,
        )
        .unwrap();
        let a = r.specs[0].approval.as_ref().unwrap();
        assert_eq!((a.standing.as_str(), a.note.as_deref()), ("rejected", None));
        assert_eq!(recheck(&r.specs[0]), Some("keel gate g1 a"));
    }

    #[test]
    fn a_waiting_approval_is_unchanged() {
        let s = with_approval(spec("login", "plan_approval"), "plan", "absent", None);
        assert_eq!(recheck(&s), None);
        assert!(!is_automatic(&s));
        let r = report(vec![s]);
        let a = pick_active(&r, None, &[]).unwrap();
        let text = render("p", None, &r, Some(&a)).join("\n");
        assert!(text.contains("Next:  moor approve"), "{text}");
    }

    #[test]
    fn a_rejected_plan_says_who_why_and_how_to_revise() {
        let s = with_approval(
            spec("login", "plan_approval"),
            "plan",
            "rejected",
            Some("needs a test task"),
        );
        assert!(is_automatic(&s), "moor go runs the re-check");
        let r = report(vec![s]);
        let a = pick_active(&r, None, &[]).unwrap();
        let text = render("p", None, &r, Some(&a)).join("\n");
        assert!(
            text.contains("step 5 of 7: approve the plan (rejected)"),
            "{text}"
        );
        assert!(
            text.contains("The plan was rejected by Dane Balia: \"needs a test task\""),
            "{text}"
        );
        assert!(text.contains("Revise:  moor ask --role build"), "{text}");
        assert!(text.contains("Then:    moor go"), "{text}");
        assert!(text.contains("Then:    moor approve"), "{text}");
        assert!(!text.contains("moor reject"), "{text}");
    }

    #[test]
    fn a_rejected_spec_is_revised_by_pushing_it_again() {
        let s = with_approval(spec("login", "spec_approval"), "spec", "rejected", None);
        let r = report(vec![s]);
        let a = pick_active(&r, None, &[]).unwrap();
        let text = render("p", None, &r, Some(&a)).join("\n");
        assert!(text.contains("with no reason given"), "{text}");
        assert!(text.contains("Revise:  moor spec push <file>"), "{text}");
    }

    #[test]
    fn a_stale_approval_is_checked_again_then_approved() {
        let s = with_approval(spec("login", "plan_approval"), "plan", "superseded", None);
        let r = report(vec![s]);
        let a = pick_active(&r, None, &[]).unwrap();
        let text = render("p", None, &r, Some(&a)).join("\n");
        assert!(text.contains("(changed since it was approved)"), "{text}");
        assert!(text.contains("Next:    moor go"), "{text}");
        assert!(!text.contains("Revise:"), "{text}");
    }

    #[test]
    fn sandbox_text_never_reaches_a_suggested_command() {
        let evil = "x\" ; rm -rf ~ ; echo \"\u{1b}[2J";
        let s = with_approval(
            spec("login", "plan_approval"),
            "plan",
            "rejected",
            Some(evil),
        );
        let r = report(vec![s]);
        let a = pick_active(&r, None, &[]).unwrap();
        for line in render("p", None, &r, Some(&a)) {
            assert!(!line.contains('\u{1b}'), "escape code printed: {line:?}");
            if line.contains("moor ") {
                assert!(
                    !line.contains("rm -rf"),
                    "note leaked into a command: {line}"
                );
            }
        }
    }

    fn report(specs: Vec<NextSpec>) -> NextReport {
        NextReport {
            blockers: vec![],
            specs,
        }
    }

    #[test]
    fn parses_keel_next_json() {
        let r: NextReport = serde_json::from_str(
            r#"{"schema":"keel.next/1","blockers":[{"reason":"store drift","command":"keel store render"}],
               "specs":[{"slug":"a","stage":"plan_gate","command":"keel gate g1 a","complete":false}]}"#,
        )
        .unwrap();
        assert_eq!(r.blockers[0].reason, "store drift");
        assert_eq!(r.specs[0].stage, "plan_gate");
    }

    #[test]
    fn host_command_runs_on_the_host() {
        assert_eq!(
            host_command("keel approve a --stage spec", None),
            "moor keel approve a --stage spec"
        );
        assert_eq!(
            host_command("keel run a", Some("myapp")),
            "moor keel --project myapp run a"
        );
    }

    #[test]
    fn a_live_pin_wins() {
        let r = report(vec![spec("a", "run"), spec("b", "spec_approval")]);
        let a = pick_active(&r, Some("b"), &[]).unwrap();
        assert_eq!(a.spec.slug, "b");
        assert_eq!(a.why, Why::Pinned);
    }

    #[test]
    fn a_finished_pin_falls_through_and_is_named() {
        let r = report(vec![spec("a", "run"), spec("b", "complete")]);
        let a = pick_active(&r, Some("b"), &[]).unwrap();
        assert_eq!(a.spec.slug, "a");
        assert_eq!(a.stale_pin.as_deref(), Some("b"));
    }

    #[test]
    fn the_last_touched_spec_beats_a_further_one() {
        let r = report(vec![spec("far", "run"), spec("near", "spec_approval")]);
        let chain = vec![
            r#"{"kind":"exec","data":{"argv":["keel","run","far"]}}"#.to_string(),
            r#"{"kind":"gate","data":{"gate":"G0","spec":"near"}}"#.to_string(),
        ];
        let a = pick_active(&r, None, &chain).unwrap();
        assert_eq!(a.spec.slug, "near");
        assert_eq!(a.why, Why::MostRecent);
    }

    #[test]
    fn a_slug_inside_a_longer_name_does_not_count() {
        let r = report(vec![spec("login", "run"), spec("other", "spec_approval")]);
        let chain = vec![r#"{"data":{"argv":["keel","run","login-v2"]}}"#.to_string()];
        let a = pick_active(&r, None, &chain).unwrap();
        assert_eq!(a.why, Why::FurthestAlong);
        assert_eq!(a.spec.slug, "login");
    }

    #[test]
    fn nothing_open_means_no_active_spec() {
        let r = report(vec![spec("a", "complete")]);
        assert!(pick_active(&r, None, &[]).is_none());
        let lines = render("myapp", None, &r, None);
        assert!(lines
            .iter()
            .any(|l| l.contains("Next:  moor spec new <name>")));
    }

    #[test]
    fn approval_steps_offer_read_approve_and_reject() {
        let r = report(vec![spec("login", "plan_approval")]);
        let a = pick_active(&r, None, &[]).unwrap();
        let text = render("myapp", None, &r, Some(&a)).join("\n");
        assert!(text.contains("myapp · login · step 5 of 7: approve the plan"));
        assert!(text.contains("Next:  moor approve      (shows the plan first, then asks)"));
        assert!(text.contains("Or:    moor reject \"why\""));
        assert!(!text.contains("keel"), "{text}");
    }

    #[test]
    fn gate_steps_say_where_to_look_if_it_fails() {
        let r = report(vec![spec("login", "plan_gate")]);
        let a = pick_active(&r, None, &[]).unwrap();
        let text = render("myapp", None, &r, Some(&a)).join("\n");
        assert!(text.contains("Next:  moor go"));
        assert!(text.contains("If it fails:  moor view login plan"));
        assert!(!text.contains("reject"));
    }

    #[test]
    fn every_printed_command_is_a_moor_command() {
        for stage in [
            "spec",
            "spec_approval",
            "plan",
            "plan_gate",
            "plan_approval",
            "run",
            "merge_approval",
        ] {
            let r = report(vec![spec("s", stage)]);
            let a = pick_active(&r, None, &[]).unwrap();
            for line in render("p", Some("p"), &r, Some(&a)) {
                if let Some((_, cmd)) = line.split_once(":  ") {
                    let cmd = cmd.trim_start();
                    assert!(cmd.starts_with("moor "), "{stage}: `{line}`");
                    assert!(cmd.contains("--project p"), "{stage}: `{line}`");
                }
            }
        }
    }

    #[test]
    fn other_open_specs_are_counted_not_listed() {
        let r = report(vec![
            spec("a", "run"),
            spec("b", "spec_approval"),
            spec("c", "complete"),
        ]);
        let a = pick_active(&r, None, &[]).unwrap();
        let text = render("p", None, &r, Some(&a)).join("\n");
        assert!(text.contains("1 other spec in progress"));
        assert!(!text.contains(" b "));
    }

    #[test]
    fn blockers_come_first_with_a_host_fix() {
        let mut r = report(vec![spec("a", "run")]);
        r.blockers.push(Blocker {
            reason: "store drift".into(),
            command: "keel store render".into(),
        });
        let a = pick_active(&r, None, &[]).unwrap();
        let lines = render("p", None, &r, Some(&a));
        assert_eq!(lines[0], "Blocked: store drift.");
        assert_eq!(lines[1], "  Fix:  moor keel store render");
    }

    #[test]
    fn render_all_marks_the_active_spec() {
        let r = report(vec![spec("a", "run"), spec("bb", "complete")]);
        let a = pick_active(&r, None, &[]).unwrap();
        let lines = render_all(&r, Some(&a));
        assert_eq!(lines[0], "▸ a   step 6 of 7: build it");
        assert_eq!(lines[1], "  bb  done");
    }
}
