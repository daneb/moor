//! What the console knows, and what a keypress does to it. No I/O and no
//! `docker` anywhere in this file: everything effectful arrives through
//! `super::Host`, which is what lets the whole interaction be asserted on
//! without a terminal or a container.

use anyhow::Result;
use serde::Deserialize;

/// One line of the visible exchange for a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Operator(String),
    Agent(String),
    Note(String),
}

impl Project {
    pub fn pending(&self) -> bool {
        self.pending_since.is_some()
    }

    /// How long the in-flight turn has been running.
    pub fn waited(&self) -> Option<std::time::Duration> {
        self.pending_since.map(|t| t.elapsed())
    }
}

impl Line {
    pub fn render(&self) -> String {
        match self {
            Line::Operator(t) => format!("> {t}"),
            Line::Agent(t) => t.clone(),
            Line::Note(t) => format!("-- {t}"),
        }
    }
}

#[derive(Debug, Default)]
pub struct Project {
    pub name: String,
    pub up: bool,
    /// Whatever `keel next --json` reported, or None. Never inferred —
    /// see `apply_next`.
    pub slug: Option<String>,
    pub stage: Option<String>,
    pub next_command: Option<String>,
    /// When the in-flight turn was sent, if one is. An instant rather than
    /// a flag because "awaiting a response" that never changes is
    /// indistinguishable, to the operator, from a console that has hung —
    /// a real turn runs for a minute or two, so the elapsed time is the
    /// feedback.
    pub pending_since: Option<std::time::Instant>,
    /// Per-project, so switching projects cannot carry a half-typed
    /// question into another repository.
    pub input: String,
    pub transcript: Vec<Line>,
    /// Names of the checks the project's last gate run failed, read from
    /// keel's own gate evidence. Empty when the stage is not a failed
    /// gate — never inferred from anything else.
    pub failing_checks: Vec<String>,
}

/// Approval is a two-step, and the steps are different keys. A single
/// keystroke — including a repeated one, including a held-down one — can
/// never advance a stage. `keel approve` is the human checkpoint the whole
/// pipeline is built around; `agent-session-protocol` took it out of the
/// agent's reach, and this keeps it out of a slip's reach.
#[derive(Debug, PartialEq, Eq)]
pub enum Approval {
    Idle,
    /// Collecting the reason for a rejection. keel records a note with a
    /// rejection, so this cannot be a single keypress the way a cancel
    /// could be.
    Rejecting {
        project: String,
        slug: String,
        stage: String,
        command: Vec<String>,
        reason: String,
    },
    /// A rejection with a reason, waiting on the confirming key.
    RejectArmed {
        project: String,
        slug: String,
        stage: String,
        command: Vec<String>,
        reason: String,
    },
    Armed {
        project: String,
        slug: String,
        stage: String,
        /// keel's own approve command for this stage, taken verbatim from
        /// `keel next --json` rather than reassembled from the stage name.
        command: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Up,
    Down,
    Esc,
}

/// What the event loop should go and do. The state machine decides;
/// `super` performs.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Refresh,
    SendTurn {
        project: String,
        prompt: String,
        /// The session to resume, read from that project's own stored id.
        resume: Option<String>,
    },
    Approve {
        project: String,
        slug: String,
        command: Vec<String>,
    },
    Reject {
        project: String,
        slug: String,
        command: Vec<String>,
        note: String,
    },
    EditArtifact {
        project: String,
        slug: String,
        artifact: String,
    },
}

/// Matches `keel next --json`'s "schema": "keel.next/1" — the same shape
/// `commands/recipe.rs` parses. Declared again here rather than shared
/// because that module's copy is private to the recipe runner and
/// `recipe.rs` is outside this change's scope; both read the same two
/// fields off the same command.
#[derive(Debug, Deserialize)]
struct NextSpec {
    slug: String,
    stage: String,
    command: String,
    #[serde(default)]
    complete: bool,
}

#[derive(Debug, Deserialize, Default)]
struct NextReport {
    #[serde(default)]
    specs: Vec<NextSpec>,
}

#[derive(Debug)]
pub struct Console {
    pub projects: Vec<Project>,
    pub selected: usize,
    pub approval: Approval,
    pub status: String,
    /// Session ids, indexed the same way `projects` is. Held per project
    /// and never merged: see `switching_projects_isolates_sessions`.
    sessions: Vec<Option<String>>,
    /// Which artifact `e` opens, cycling spec -> plan -> tasks.
    artifact: usize,
}

const ARTIFACTS: [&str; 3] = ["spec", "plan", "tasks"];

impl Console {
    pub fn new(names: &[String]) -> Console {
        Console {
            projects: names
                .iter()
                .map(|n| Project {
                    name: n.clone(),
                    ..Default::default()
                })
                .collect(),
            selected: 0,
            approval: Approval::Idle,
            status: String::new(),
            sessions: vec![None; names.len()],
            artifact: 0,
        }
    }

    pub fn selected_project(&self) -> Option<&Project> {
        self.projects.get(self.selected)
    }

    fn index_of(&self, project: &str) -> Option<usize> {
        self.projects.iter().position(|p| p.name == project)
    }

    /// Bring live container state and per-spec stage up to date.
    ///
    /// One `docker ps` for every project, not one per project: `Host`
    /// exposes exactly one bulk query, so the refresh cost is flat in the
    /// number of projects on disk. `keel next` is then asked only of the
    /// sandboxes that are actually up — a stopped project has no container
    /// to ask.
    pub fn refresh(&mut self, host: &dyn super::Host) -> Result<()> {
        let names: Vec<String> = self.projects.iter().map(|p| p.name.clone()).collect();
        let running = host.running_projects(&names)?;
        for p in &mut self.projects {
            p.up = running.contains(&p.name);
        }
        for i in 0..self.projects.len() {
            let name = self.projects[i].name.clone();
            self.set_session(&name, host.stored_session(&name));
            if !self.projects[i].up {
                continue;
            }
            match host.keel_next(&name) {
                Ok(json) => self.apply_next(&name, &json)?,
                Err(e) => self.status = format!("couldn't read {name}'s pipeline status: {e}"),
            }
            // Only for a spec whose stage is a gate: everything else has no
            // failing checks to show, and reading evidence for it would be
            // a wasted exec per refresh.
            let slug = self.projects[i].slug.clone();
            let at_gate = self.projects[i]
                .stage
                .as_deref()
                .is_some_and(|s| s.contains("gate"));
            self.projects[i].failing_checks = match (slug, at_gate) {
                (Some(slug), true) => host
                    .gate_output(&name, &slug)
                    .map(|json| failing_checks(&json))
                    .unwrap_or_default(),
                _ => vec![],
            };
        }
        Ok(())
    }

    /// Take stage and next command straight from keel's own report. If
    /// keel names no spec, the console says so rather than guessing from
    /// what files happen to exist — a displayed stage that disagrees with
    /// keel is worse than no stage at all.
    pub fn apply_next(&mut self, project: &str, json: &str) -> Result<()> {
        let report: NextReport = serde_json::from_str(json.trim()).unwrap_or_default();
        let Some(i) = self.index_of(project) else {
            return Ok(());
        };
        match report.specs.first() {
            Some(spec) => {
                self.projects[i].slug = Some(spec.slug.clone());
                self.projects[i].stage = Some(if spec.complete {
                    "complete".to_string()
                } else {
                    spec.stage.clone()
                });
                self.projects[i].next_command = Some(spec.command.clone());
            }
            None => {
                self.projects[i].slug = None;
                self.projects[i].stage = None;
                self.projects[i].next_command = None;
            }
        }
        Ok(())
    }

    /// Seed what keel would have reported, without asking it.
    #[cfg(test)]
    pub fn set_stage(
        &mut self,
        project: &str,
        slug: Option<String>,
        stage: Option<String>,
        command: Option<String>,
    ) {
        if let Some(i) = self.index_of(project) {
            self.projects[i].slug = slug;
            self.projects[i].stage = stage;
            self.projects[i].next_command = command;
        }
    }

    pub fn set_session(&mut self, project: &str, id: Option<String>) {
        if let Some(i) = self.index_of(project) {
            self.sessions[i] = id;
        }
    }

    #[cfg(test)]
    pub fn set_input(&mut self, project: &str, text: &str) {
        if let Some(i) = self.index_of(project) {
            self.projects[i].input = text.to_string();
        }
    }

    pub fn begin_turn(&mut self, project: &str) {
        if let Some(i) = self.index_of(project) {
            self.projects[i].pending_since = Some(std::time::Instant::now());
        }
    }

    pub fn complete_turn(&mut self, project: &str, text: &str, failed: bool) {
        if let Some(i) = self.index_of(project) {
            self.projects[i].pending_since = None;
            if failed {
                self.projects[i].transcript.push(Line::Note(
                    "the agent reported an error for this turn".into(),
                ));
            }
            self.projects[i]
                .transcript
                .push(Line::Agent(text.to_string()));
        }
    }

    pub fn note(&mut self, project: &str, text: &str) {
        if let Some(i) = self.index_of(project) {
            self.projects[i]
                .transcript
                .push(Line::Note(text.to_string()));
        }
    }

    /// Arm an approval against the *currently selected* project and the
    /// slug keel itself reported for it, captured now so a later selection
    /// change cannot redirect the confirmation at something else.
    pub fn arm_approval(&mut self) {
        self.arm_decision(false);
    }

    /// Arm a decision on whatever keel says this project is waiting on,
    /// capturing the slug and the command keel itself reported so a later
    /// selection change cannot redirect the confirmation at something
    /// else. `reject` picks which state it lands in; the resolution and
    /// every refusal are identical, which is why they share this.
    fn arm_decision(&mut self, reject: bool) {
        let verb = if reject { "reject" } else { "approve" };
        let Some(p) = self.selected_project() else {
            return;
        };
        let command: Vec<String> = p
            .next_command
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .map(String::from)
            .collect();
        // Only armable when keel itself says the next step is an approval.
        // If keel wants a gate or a run next, there is nothing here for a
        // keystroke to advance.
        let is_approve = command.first().map(String::as_str) == Some("keel")
            && command.get(1).map(String::as_str) == Some("approve");
        match (&p.slug, &p.stage, is_approve) {
            (Some(slug), Some(stage), true) => {
                let (project, slug, stage) = (p.name.clone(), slug.clone(), stage.clone());
                self.approval = if reject {
                    self.status = "why reject? type a reason, then Enter".into();
                    Approval::Rejecting {
                        project,
                        slug,
                        stage,
                        command,
                        reason: String::new(),
                    }
                } else {
                    Approval::Armed {
                        project,
                        slug,
                        stage,
                        command,
                    }
                };
            }
            (_, _, false) if p.slug.is_some() => {
                self.status = format!(
                    "keel's next step here is not an approval: {}",
                    p.next_command.clone().unwrap_or_else(|| "unknown".into())
                )
            }
            _ => self.status = format!("nothing to {verb}: keel reports no active spec here"),
        }
    }

    /// Keys while a rejection's reason is being typed. Enter arms it, but
    /// only with a reason: keel records the note, and an empty one tells
    /// the next attempt nothing.
    fn key_while_rejecting(&mut self, key: Key) -> Action {
        let Approval::Rejecting {
            project,
            slug,
            stage,
            command,
            reason,
        } = &mut self.approval
        else {
            return Action::None;
        };
        match key {
            Key::Esc => {
                self.approval = Approval::Idle;
                self.status = "rejection abandoned".into();
            }
            Key::Backspace => {
                reason.pop();
            }
            Key::Char(c) => reason.push(c),
            Key::Enter => {
                if reason.trim().is_empty() {
                    self.status = "a rejection needs a reason — type why, then Enter".into();
                    return Action::None;
                }
                let armed = Approval::RejectArmed {
                    project: project.clone(),
                    slug: slug.clone(),
                    stage: stage.clone(),
                    command: command.clone(),
                    reason: reason.trim().to_string(),
                };
                self.approval = armed;
                self.status = "press y to confirm the rejection".into();
            }
            Key::Up | Key::Down => {}
        }
        Action::None
    }

    /// Begin a rejection: same resolution as `arm_approval`, different
    /// landing state.
    pub fn arm_rejection(&mut self) {
        self.arm_decision(true);
    }

    pub fn handle_key(&mut self, key: Key) -> Action {
        // A rejection in progress owns the keyboard: its reason is typed,
        // so ordinary text keys must not reach the agent input, and Esc
        // abandons it rather than quitting the console.
        if let Approval::Rejecting { .. } = &self.approval {
            return self.key_while_rejecting(key);
        }
        if let Approval::RejectArmed {
            project,
            slug,
            stage,
            command,
            reason,
        } = &self.approval
        {
            let armed = (
                project.clone(),
                slug.clone(),
                stage.clone(),
                command.clone(),
                reason.clone(),
            );
            self.approval = Approval::Idle;
            if key == Key::Char('y') {
                self.status = format!("rejecting {} stage of '{}'", armed.2, armed.1);
                return Action::Reject {
                    project: armed.0,
                    slug: armed.1,
                    command: armed.3,
                    note: armed.4,
                };
            }
            self.status = "rejection cancelled".into();
            return Action::None;
        }
        // An armed approval consumes the next key, whatever it is. Only
        // `y` confirms; everything else — including another `a` — cancels,
        // so no repeated keystroke can approve a stage.
        if let Approval::Armed {
            project,
            slug,
            stage,
            command,
        } = &self.approval
        {
            let armed = (
                project.clone(),
                slug.clone(),
                stage.clone(),
                command.clone(),
            );
            self.approval = Approval::Idle;
            if key == Key::Char('y') {
                self.status = format!("approving {} stage of '{}'", armed.2, armed.1);
                return Action::Approve {
                    project: armed.0,
                    slug: armed.1,
                    command: armed.3,
                };
            }
            self.status = "approval cancelled".into();
            return Action::None;
        }

        self.status.clear();
        match key {
            Key::Up => {
                self.selected = self.selected.saturating_sub(1);
                Action::None
            }
            Key::Down => {
                if self.selected + 1 < self.projects.len() {
                    self.selected += 1;
                }
                Action::None
            }
            Key::Esc => Action::Quit,
            Key::Backspace => {
                if let Some(i) = self.projects.get_mut(self.selected) {
                    i.input.pop();
                }
                Action::None
            }
            Key::Char(c) => self.handle_char(c),
            Key::Enter => self.send(),
        }
    }

    fn handle_char(&mut self, c: char) -> Action {
        let typing = self.selected_project().is_some_and(|p| !p.input.is_empty());
        // Commands are only commands on an empty input line; once the
        // operator is mid-question, every key is text.
        if !typing {
            match c {
                'q' => return Action::Quit,
                'r' => return Action::Refresh,
                'a' => {
                    self.arm_approval();
                    return Action::None;
                }
                'x' => {
                    self.arm_rejection();
                    return Action::None;
                }
                'e' => return self.edit(),
                _ => {}
            }
        }
        if let Some(p) = self.projects.get_mut(self.selected) {
            p.input.push(c);
        }
        Action::None
    }

    fn edit(&mut self) -> Action {
        let artifact = ARTIFACTS[self.artifact % ARTIFACTS.len()].to_string();
        self.artifact += 1;
        match self.selected_project() {
            Some(p) => match &p.slug {
                Some(slug) => Action::EditArtifact {
                    project: p.name.clone(),
                    slug: slug.clone(),
                    artifact,
                },
                None => {
                    self.status = "nothing to edit: keel reports no active spec here".into();
                    Action::None
                }
            },
            None => Action::None,
        }
    }

    fn send(&mut self) -> Action {
        let Some(i) = self.projects.checked_index(self.selected) else {
            return Action::None;
        };
        let prompt = self.projects[i].input.trim().to_string();
        if prompt.is_empty() {
            return Action::None;
        }
        if self.projects[i].pending() {
            self.status = "that project is still awaiting a response".into();
            return Action::None;
        }
        if !self.projects[i].up {
            self.status = format!(
                "{} is not up — `moor up {}` first",
                self.projects[i].name, self.projects[i].name
            );
            return Action::None;
        }
        self.projects[i].input.clear();
        self.projects[i]
            .transcript
            .push(Line::Operator(prompt.clone()));
        let name = self.projects[i].name.clone();
        self.begin_turn(&name);
        Action::SendTurn {
            project: self.projects[i].name.clone(),
            prompt,
            // This project's own stored session and nothing else — the
            // agent in one repository never receives another's context.
            resume: self.sessions[i].clone(),
        }
    }
}

/// The names of the checks a keel gate record reports as failed. Pure, so
/// it is tested against real gate JSON without a container. Anything that
/// is not a recognisable gate record yields no names rather than an error:
/// a missing or half-written evidence file is not worth failing a refresh
/// over.
pub fn failing_checks(gate_json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(gate_json.trim()) else {
        return vec![];
    };
    let checks = v
        .get("checks")
        .and_then(|c| c.as_array())
        .map(|a| a.as_slice())
        .unwrap_or_default();
    // keel's gate record names each check by `id` and its outcome by
    // `verdict` — checked against a real G1.json rather than assumed.
    // `detail` carries why, which is the part the operator actually needs.
    checks
        .iter()
        .filter(|c| {
            matches!(
                c.get("verdict").and_then(|s| s.as_str()),
                Some("fail") | Some("blocked")
            )
        })
        .filter_map(|c| {
            let id = c.get("id").and_then(|n| n.as_str())?;
            Ok::<String, ()>(match c.get("detail").and_then(|d| d.as_str()) {
                Some(d) if !d.trim().is_empty() => format!("{id}: {d}"),
                _ => id.to_string(),
            })
            .ok()
        })
        .collect()
}

/// Tiny helper so `send` can bail cleanly on an empty project list.
trait CheckedIndex {
    fn checked_index(&self, i: usize) -> Option<usize>;
}
impl CheckedIndex for Vec<Project> {
    fn checked_index(&self, i: usize) -> Option<usize> {
        (i < self.len()).then_some(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::tests::FakeHost;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// AC-1
    #[test]
    fn refresh_issues_one_container_query() {
        let many = names(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"]);
        let host = FakeHost::new().with_running(&["c", "j"]).with_next(
            r#"{"schema":"keel.next/1","specs":[{"slug":"s","stage":"plan","command":"keel plan s","complete":false}]}"#,
        );
        let mut c = Console::new(&many);
        c.refresh(&host).unwrap();

        assert_eq!(
            host.container_queries(),
            1,
            "12 projects must still cost exactly one `docker ps`"
        );
        // ...and that one query was asked about every project, not just one.
        assert_eq!(host.last_query_names(), many);
        assert!(c.projects.iter().filter(|p| p.up).count() == 2);

        // Adding projects does not add queries.
        let host2 = FakeHost::new().with_running(&[]);
        Console::new(&names(&["x"])).refresh(&host2).unwrap();
        assert_eq!(host2.container_queries(), 1);
        // keel next is asked only of what is actually up.
        assert_eq!(host2.keel_next_calls(), 0);
    }

    /// AC-4
    #[test]
    fn approval_requires_a_second_distinct_key() {
        let mut c = Console::new(&names(&["demo"]));
        c.set_stage(
            "demo",
            Some("blast-radius".into()),
            Some("plan-approval".into()),
            Some("keel approve blast-radius --stage plan".into()),
        );
        let approve_argv: Vec<String> = "keel approve blast-radius --stage plan"
            .split(' ')
            .map(String::from)
            .collect();

        // One key only arms it, and says exactly what it would approve.
        assert_eq!(c.handle_key(Key::Char('a')), Action::None);
        assert_eq!(
            c.approval,
            Approval::Armed {
                project: "demo".into(),
                slug: "blast-radius".into(),
                stage: "plan-approval".into(),
                command: approve_argv.clone(),
            }
        );

        // The *same* key again must not confirm — a held or double-tapped
        // key is the obvious way to approve something by accident.
        assert_eq!(c.handle_key(Key::Char('a')), Action::None);
        assert_eq!(c.approval, Approval::Idle);
        assert_eq!(c.status, "approval cancelled");

        // Nor does any other key, and each one disarms.
        for key in [
            Key::Enter,
            Key::Esc,
            Key::Up,
            Key::Char('Y'),
            Key::Char('n'),
            Key::Char('q'),
        ] {
            c.handle_key(Key::Char('a'));
            assert!(matches!(c.approval, Approval::Armed { .. }));
            assert_ne!(
                c.handle_key(key),
                Action::Approve {
                    project: "demo".into(),
                    slug: "blast-radius".into(),
                    command: approve_argv.clone(),
                },
                "{key:?} must not approve"
            );
            assert_eq!(c.approval, Approval::Idle);
        }

        // Only arm-then-y issues the verb, naming its target.
        c.handle_key(Key::Char('a'));
        assert_eq!(
            c.handle_key(Key::Char('y')),
            Action::Approve {
                project: "demo".into(),
                slug: "blast-radius".into(),
                command: approve_argv,
            }
        );
        assert_eq!(c.approval, Approval::Idle);

        // A stage keel is not asking a human about cannot be armed at all.
        c.set_stage(
            "demo",
            Some("blast-radius".into()),
            Some("build".into()),
            Some("keel run blast-radius".into()),
        );
        c.handle_key(Key::Char('a'));
        assert_eq!(c.approval, Approval::Idle);
        assert!(c.status.contains("not an approval"));

        // With no spec of keel's to name, there is nothing to arm.
        let mut empty = Console::new(&names(&["bare"]));
        empty.handle_key(Key::Char('a'));
        assert_eq!(empty.approval, Approval::Idle);
        assert!(empty.status.contains("no active spec"));
    }

    /// AC-6
    #[test]
    fn switching_projects_isolates_sessions() {
        let mut c = Console::new(&names(&["alpha", "beta"]));
        for p in &mut c.projects {
            p.up = true;
        }
        c.set_session("alpha", Some("aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa".into()));
        c.set_session("beta", Some("bbbbbbbb-2222-2222-2222-bbbbbbbbbbbb".into()));

        // Ask alpha something.
        for ch in "secret alpha context".chars() {
            c.handle_key(Key::Char(ch));
        }
        let first = c.handle_key(Key::Enter);
        assert_eq!(
            first,
            Action::SendTurn {
                project: "alpha".into(),
                prompt: "secret alpha context".into(),
                resume: Some("aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa".into()),
            }
        );

        // Switch to beta and ask it something.
        c.handle_key(Key::Down);
        for ch in "unrelated beta question".chars() {
            c.handle_key(Key::Char(ch));
        }
        let second = c.handle_key(Key::Enter);
        let Action::SendTurn {
            project,
            prompt,
            resume,
        } = second
        else {
            panic!("expected a turn for beta, got {second:?}");
        };
        assert_eq!(project, "beta");
        // Beta's turn resumes beta's own session...
        assert_eq!(
            resume.as_deref(),
            Some("bbbbbbbb-2222-2222-2222-bbbbbbbbbbbb")
        );
        // ...and carries no part of alpha's conversation.
        assert_eq!(prompt, "unrelated beta question");
        assert!(!prompt.contains("alpha"));
        assert!(!prompt.contains("aaaaaaaa"));

        // A reply to one project never lands in the other's transcript.
        c.complete_turn("alpha", "alpha's answer", false);
        let beta = c.projects.iter().find(|p| p.name == "beta").unwrap();
        assert!(!beta
            .transcript
            .iter()
            .any(|l| l.render().contains("alpha's answer")));

        // A half-typed question stays with the project it was typed in.
        c.handle_key(Key::Up);
        for ch in "half typed".chars() {
            c.handle_key(Key::Char(ch));
        }
        c.handle_key(Key::Down);
        assert_eq!(
            c.projects.iter().find(|p| p.name == "beta").unwrap().input,
            ""
        );
        assert_eq!(
            c.projects.iter().find(|p| p.name == "alpha").unwrap().input,
            "half typed"
        );
    }

    /// AC-7
    #[test]
    fn stage_is_read_from_keel_next() {
        let mut c = Console::new(&names(&["demo"]));
        let report = r#"{"schema":"keel.next/1","specs":[
            {"slug":"blast-radius","stage":"plan-approval","command":"keel approve blast-radius --stage plan","complete":false}]}"#;
        c.apply_next("demo", report).unwrap();
        let p = &c.projects[0];
        // Exactly what keel said, not a re-derivation of it.
        assert_eq!(p.slug.as_deref(), Some("blast-radius"));
        assert_eq!(p.stage.as_deref(), Some("plan-approval"));
        assert_eq!(
            p.next_command.as_deref(),
            Some("keel approve blast-radius --stage plan")
        );
        // And what's displayed is that same string.
        assert!(crate::studio::render::frame(&c, 100, 24).contains("plan-approval"));

        // keel naming no spec means the console shows none — it does not
        // fall back to guessing from whatever is on disk.
        c.apply_next("demo", r#"{"schema":"keel.next/1","specs":[]}"#)
            .unwrap();
        assert_eq!(c.projects[0].stage, None);
        assert_eq!(c.projects[0].slug, None);
        assert!(crate::studio::render::frame(&c, 100, 24).contains("no active spec"));

        // A complete spec reports as complete, from keel's own flag.
        c.apply_next(
            "demo",
            r#"{"specs":[{"slug":"done-thing","stage":"g4","command":"keel gate g4 done-thing","complete":true}]}"#,
        )
        .unwrap();
        assert_eq!(c.projects[0].stage.as_deref(), Some("complete"));

        // Unparseable output leaves no invented stage behind either.
        c.apply_next("demo", "not json at all").unwrap();
        assert_eq!(c.projects[0].stage, None);
    }

    #[test]
    fn typed_text_wins_over_command_keys_once_a_question_is_started() {
        let mut c = Console::new(&names(&["demo"]));
        c.projects[0].up = true;
        // 'q' on an empty line quits; inside a word it is just a letter.
        assert_eq!(c.handle_key(Key::Char('q')), Action::Quit);
        for ch in "what does q".chars() {
            c.handle_key(Key::Char(ch));
        }
        assert_eq!(c.projects[0].input, "what does q");
        assert_eq!(c.handle_key(Key::Char('r')), Action::None);
        assert_eq!(c.projects[0].input, "what does qr");
    }

    #[test]
    fn a_stopped_project_refuses_a_turn_instead_of_hanging_on_docker() {
        let mut c = Console::new(&names(&["down-project"]));
        for ch in "anything".chars() {
            c.handle_key(Key::Char(ch));
        }
        assert_eq!(c.handle_key(Key::Enter), Action::None);
        assert!(c.status.contains("not up"));
    }

    #[test]
    fn edit_cycles_the_three_keel_artifacts() {
        let mut c = Console::new(&names(&["demo"]));
        c.set_stage("demo", Some("s".into()), Some("plan".into()), None);
        let mut seen = vec![];
        for _ in 0..3 {
            if let Action::EditArtifact { artifact, .. } = c.handle_key(Key::Char('e')) {
                seen.push(artifact);
            }
        }
        assert_eq!(seen, vec!["spec", "plan", "tasks"]);
    }
    // --- studio-decisions (SPEC-0013) ----------------------------------

    /// A console with one project sitting on an approval keel reported.
    fn awaiting_decision() -> Console {
        let mut c = Console::new(&["alpha".to_string()]);
        c.apply_next(
            "alpha",
            r#"{"specs":[{"slug":"login","stage":"spec_approval","complete":false,
                 "command":"keel approve login --stage spec"}]}"#,
        )
        .unwrap();
        c
    }

    #[test]
    fn reject_key_collects_a_reason() {
        // AC-1
        let mut c = awaiting_decision();
        assert_eq!(c.handle_key(Key::Char('x')), Action::None);
        assert!(matches!(c.approval, Approval::Rejecting { .. }));
        for ch in "too vague".chars() {
            assert_eq!(c.handle_key(Key::Char(ch)), Action::None);
        }
        match &c.approval {
            Approval::Rejecting { reason, .. } => assert_eq!(reason, "too vague"),
            other => panic!("expected Rejecting, got {other:?}"),
        }
        // The typed reason did not leak into the agent input line.
        assert_eq!(c.selected_project().unwrap().input, "");
    }

    #[test]
    fn empty_reason_records_no_rejection() {
        // AC-2
        let mut c = awaiting_decision();
        c.handle_key(Key::Char('x'));
        assert_eq!(c.handle_key(Key::Enter), Action::None);
        // Still collecting, nothing emitted, and it says why.
        assert!(matches!(c.approval, Approval::Rejecting { .. }));
        assert!(c.status.contains("needs a reason"), "{}", c.status);
    }

    #[test]
    fn rejection_needs_a_second_key() {
        // AC-3: Enter arms; only `y` records.
        let mut c = awaiting_decision();
        c.handle_key(Key::Char('x'));
        for ch in "nope".chars() {
            c.handle_key(Key::Char(ch));
        }
        assert_eq!(c.handle_key(Key::Enter), Action::None);
        assert!(matches!(c.approval, Approval::RejectArmed { .. }));
        match c.handle_key(Key::Char('y')) {
            Action::Reject { slug, note, .. } => {
                assert_eq!(slug, "login");
                assert_eq!(note, "nope");
            }
            other => panic!("expected Reject, got {other:?}"),
        }
        // A non-y key cancels instead of recording.
        let mut c2 = awaiting_decision();
        c2.handle_key(Key::Char('x'));
        c2.handle_key(Key::Char('z'));
        c2.handle_key(Key::Enter);
        assert_eq!(c2.handle_key(Key::Char('n')), Action::None);
        assert!(matches!(c2.approval, Approval::Idle));
    }

    #[test]
    fn escape_abandons_a_rejection() {
        // AC-4
        let mut c = awaiting_decision();
        c.handle_key(Key::Char('x'));
        c.handle_key(Key::Char('w'));
        assert_eq!(c.handle_key(Key::Esc), Action::None); // not Quit
        assert!(matches!(c.approval, Approval::Idle));
        assert!(c.status.contains("abandoned"), "{}", c.status);
    }

    #[test]
    fn rejection_uses_keels_own_command_and_note() {
        // AC-5: keel's verbatim command, with the reason as the note.
        let mut c = awaiting_decision();
        c.handle_key(Key::Char('x'));
        for ch in "needs rollback".chars() {
            c.handle_key(Key::Char(ch));
        }
        c.handle_key(Key::Enter);
        match c.handle_key(Key::Char('y')) {
            Action::Reject { command, note, .. } => {
                assert_eq!(command, vec!["keel", "approve", "login", "--stage", "spec"]);
                assert_eq!(note, "needs rollback");
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn failing_checks_are_shown() {
        // AC-6: the parser reads keel's real gate shape, and the frame
        // draws the names.
        let gate = r#"{"gate":"G1","verdict":"fail","checks":[
            {"id":"schema","verdict":"pass","detail":"ok"},
            {"id":"task-exit-conditions","verdict":"fail","detail":"no exit condition on T-1"},
            {"id":"test-movement","verdict":"blocked","detail":"confirm the oracles"}]}"#;
        let names = failing_checks(gate);
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names[0].starts_with("task-exit-conditions"), "{names:?}");
        assert!(names[1].starts_with("test-movement"), "{names:?}");
        // Not a gate record at all: no names, no error.
        assert!(failing_checks("not json").is_empty());
        assert!(failing_checks("").is_empty());

        let mut c = Console::new(&["alpha".to_string()]);
        c.projects[0].failing_checks = names;
        let drawn = crate::studio::render::frame(&c, 100, 24);
        assert!(drawn.contains("task-exit-conditions"), "{drawn}");
    }

    #[test]
    fn failing_checks_come_through_the_host_on_refresh() {
        // AC-6, at the boundary: refresh reads gate evidence via the Host
        // for a spec at a gate, and only for one at a gate.
        let gate = r#"{"gate":"G1","verdict":"fail","checks":[
            {"id":"total-budget","verdict":"fail","detail":"446 lines"}]}"#;
        let host = crate::studio::tests::FakeHost::new()
            .with_running(&["alpha"])
            .with_next(
                r#"{"specs":[{"slug":"login","stage":"plan_gate","complete":false,
                     "command":"keel gate g1 login"}]}"#,
            )
            .with_gate_json(gate);
        let mut c = Console::new(&["alpha".to_string()]);
        c.refresh(&host).unwrap();
        assert_eq!(c.projects[0].failing_checks.len(), 1);
        assert!(c.projects[0].failing_checks[0].starts_with("total-budget"));

        // A spec that is not at a gate gets no gate read at all.
        let host2 = crate::studio::tests::FakeHost::new()
            .with_running(&["alpha"])
            .with_next(
                r#"{"specs":[{"slug":"login","stage":"spec_approval","complete":false,
                     "command":"keel approve login --stage spec"}]}"#,
            )
            .with_gate_json(gate);
        let mut c2 = Console::new(&["alpha".to_string()]);
        c2.refresh(&host2).unwrap();
        assert!(c2.projects[0].failing_checks.is_empty());
    }

    #[test]
    fn gate_output_is_sanitized() {
        // AC-7: a hostile check name cannot address the terminal.
        let mut c = Console::new(&["alpha".to_string()]);
        c.projects[0].failing_checks = vec!["evil\x1b[2J\x1b]0;pwned\x07check".to_string()];
        let drawn = crate::studio::render::frame(&c, 100, 24);
        assert!(!drawn.contains("\x1b[2J"), "escape survived: {drawn:?}");
        assert!(!drawn.contains("\x1b]0;"), "OSC survived: {drawn:?}");
        assert!(drawn.contains("evil"), "text lost: {drawn}");
    }
}
