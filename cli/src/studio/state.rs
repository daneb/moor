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
}

/// Approval is a two-step, and the steps are different keys. A single
/// keystroke — including a repeated one, including a held-down one — can
/// never advance a stage. `keel approve` is the human checkpoint the whole
/// pipeline is built around; `agent-session-protocol` took it out of the
/// agent's reach, and this keeps it out of a slip's reach.
#[derive(Debug, PartialEq, Eq)]
pub enum Approval {
    Idle,
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
                Err(e) => self.status = format!("keel next failed for {name}: {e}"),
            }
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
                self.approval = Approval::Armed {
                    project: p.name.clone(),
                    slug: slug.clone(),
                    stage: stage.clone(),
                    command,
                };
            }
            (_, _, false) if p.slug.is_some() => {
                self.status = format!(
                    "keel's next step here is not an approval: {}",
                    p.next_command.clone().unwrap_or_else(|| "unknown".into())
                )
            }
            _ => self.status = "nothing to approve: keel reports no active spec here".into(),
        }
    }

    pub fn handle_key(&mut self, key: Key) -> Action {
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
}
