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
}

/// keel's pipeline has seven steps before `complete` (keel
/// `pipeline::Stage`, whose wire keys are a stable schema).
const TOTAL_STEPS: u8 = 7;

/// What one stage means for the operator.
struct Step {
    n: u8,
    title: &'static str,
    what: &'static str,
    /// The keel artifact worth reading before acting, if any.
    read: Option<&'static str>,
    /// The approval stage this step decides, so a reject can be offered.
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
            "G0 hasn't passed yet. Run it; if it fails, fix the spec and run it again.",
            Some("spec"),
            None,
        ),
        "spec_approval" => s(
            2,
            "approve the spec",
            "The spec passed G0. Read it, then approve or reject it.",
            Some("spec"),
            Some("spec"),
        ),
        "plan" => s(
            3,
            "plan the work",
            "The spec is approved. keel drafts a plan and its tasks next.",
            None,
            None,
        ),
        "plan_gate" => s(
            4,
            "check the plan",
            "A plan exists but G1 isn't passing. Run it; if it fails, read the plan to see why.",
            Some("plan"),
            None,
        ),
        "plan_approval" => s(
            5,
            "approve the plan",
            "The plan passed G1. Read it, then approve or reject it.",
            Some("plan"),
            Some("plan"),
        ),
        "run" => s(
            6,
            "build it",
            "The plan is approved. keel runs the tasks through the agent and checks the result.",
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
        (false, Some(s)) => format!("step {} of {TOTAL_STEPS}: {}", s.n, s.title),
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
        out.push("  Start one by giving it a short name (e.g. login-endpoint).".to_string());
        out.push(String::new());
        out.push(format!("  Next:  moor keel{flag} spec new <name>"));
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
        Some(s) => {
            out.push(format!("  {}", s.what));
            out.push(String::new());
            if let (Some(artifact), Some(_)) = (s.read, s.decides) {
                out.push(format!(
                    "  Read:  {}",
                    read_command(&spec.slug, artifact, explicit)
                ));
            }
            out.push(format!(
                "  Next:  {}",
                host_command(&spec.command, explicit)
            ));
            if let Some(stage) = s.decides {
                out.push(format!(
                    "  Or:    moor keel{flag} approve {} --stage {stage} --reject --note \"why\"",
                    spec.slug
                ));
            } else if let Some(artifact) = s.read {
                out.push(format!(
                    "  If it fails:  {}",
                    read_command(&spec.slug, artifact, explicit)
                ));
            }
        }
        None => {
            // A stage this moor doesn't know yet: still point at keel's
            // own next command rather than guessing.
            out.push(format!("  keel says this spec is at `{}`.", spec.stage));
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
            .any(|l| l.contains("moor keel spec new <name>")));
    }

    #[test]
    fn approval_steps_offer_read_approve_and_reject() {
        let r = report(vec![spec("login", "plan_approval")]);
        let a = pick_active(&r, None, &[]).unwrap();
        let text = render("myapp", None, &r, Some(&a)).join("\n");
        assert!(text.contains("myapp · login · step 5 of 7: approve the plan"));
        assert!(text.contains("Read:  moor view login plan"));
        assert!(text.contains("Next:  moor keel approve login --stage plan"));
        assert!(text.contains("--stage plan --reject --note"));
    }

    #[test]
    fn gate_steps_say_where_to_look_if_it_fails() {
        let r = report(vec![spec("login", "plan_gate")]);
        let a = pick_active(&r, None, &[]).unwrap();
        let text = render("myapp", None, &r, Some(&a)).join("\n");
        assert!(text.contains("Next:  moor keel gate g1 login"));
        assert!(text.contains("If it fails:  moor view login plan"));
        assert!(!text.contains("--reject"));
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
