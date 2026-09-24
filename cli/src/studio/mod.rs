//! `moor studio` — one console over every project.
//!
//! `agent-session-protocol` gives one project a chained, resumable turn,
//! driven one command at a time. Everything an operator actually does
//! spans more than that: which projects exist, which are up, what stage
//! each spec is at, and whether the plan is worth approving. All of that
//! was already cheap to get and presented nowhere at once.
//!
//! The split here is deliberate: `state.rs` decides, this file performs.
//! Every effect goes through `Host`, so the console's behaviour — one
//! container query per refresh, a turn that doesn't block input, approval
//! that takes two distinct keys — is asserted directly rather than
//! inferred from a running terminal.

pub mod render;
pub mod state;

use crate::{audit, manifest::Manifest, paths, proc, session};
use anyhow::{Context, Result};
use state::{Action, Console, Key};
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

/// Everything the console needs from outside itself. One trait, so the
/// production path and the tests drive identical logic.
pub trait Host {
    /// Which sandboxes are up — for *all* names in one call. Deliberately
    /// bulk: a per-project variant would make refresh cost grow with the
    /// number of projects on disk.
    fn running_projects(&self, names: &[String]) -> Result<Vec<String>>;
    /// Raw `keel next --json` output for a project.
    fn keel_next(&self, project: &str) -> Result<String>;
    /// That project's own stored session id, host-side.
    fn stored_session(&self, project: &str) -> Option<String>;
    fn read_artifact(&self, project: &str, slug: &str, artifact: &str) -> Result<String>;
    fn write_artifact(&self, project: &str, slug: &str, artifact: &str, local: &Path)
        -> Result<()>;
    fn approve(&self, project: &str, command: &[String]) -> Result<String>;
}

/// The three keel artifacts, and where they live inside a sandbox. Same
/// mapping `commands/view.rs` uses.
fn artifact_path(slug: &str, artifact: &str) -> Result<String> {
    // The slug reaches a child process's argv, so it passes the same rule
    // project names do before it is interpolated into a path.
    crate::manifest::validate_name(slug).context("spec slug from `keel next`")?;
    let file = match artifact {
        "spec" => "spec.md",
        "plan" => "plan.md",
        "tasks" => "tasks.md",
        other => anyhow::bail!("unknown artifact '{other}' — expected spec, plan, or tasks"),
    };
    Ok(format!("/workspace/.keel/specs/{slug}/{file}"))
}

fn read_argv(container: &str, path: &str) -> Vec<String> {
    vec![
        "exec".into(),
        container.into(),
        "cat".into(),
        path.to_string(),
    ]
}

/// The write-back argv. `docker exec -i ... sh -c 'cat > dest'` — the same
/// mechanism `proc::run_with_stdin_file` already uses to get a file into a
/// container whose rootfs is read-only. No bind mount is created, and none
/// can be: the bytes travel through the child's stdin.
fn write_back_argv(container: &str, path: &str) -> Vec<String> {
    vec![
        "exec".into(),
        "-i".into(),
        container.into(),
        "sh".into(),
        "-c".into(),
        format!("cat > {path}"),
    ]
}

/// The real host. Every command it runs is recorded through
/// `audit::log_exec` — the same chained-audit path `moor run` and `moor
/// recipe` use. The console opens no record of its own.
pub struct Docker;

impl Docker {
    fn manifest(&self, project: &str) -> Result<Manifest> {
        Manifest::load(&paths::manifest_path(project)?)
    }

    fn exec(&self, project: &str, argv: &[String]) -> Result<(bool, String)> {
        let m = self.manifest(project)?;
        let container = m.sandbox_container();
        let mut args: Vec<&str> = vec!["exec", &container];
        args.extend(argv.iter().map(String::as_str));
        let (status, out) = proc::run_capture_combined("docker", &args)?;
        audit::log_exec(project, &m, "studio", argv, status.code())?;
        Ok((status.success(), out))
    }
}

impl Host for Docker {
    fn running_projects(&self, names: &[String]) -> Result<Vec<String>> {
        // The existing one-`docker ps`-for-everything helper, not a second
        // copy of it.
        crate::commands::running_projects(names)
    }

    fn keel_next(&self, project: &str) -> Result<String> {
        let (_, out) = self.exec(project, &["keel".into(), "next".into(), "--json".into()])?;
        Ok(out)
    }

    fn stored_session(&self, project: &str) -> Option<String> {
        let path = paths::session_path(project).ok()?;
        let raw = std::fs::read_to_string(path).ok()?;
        let id = raw.trim().to_string();
        session::valid_session_id(&id).then_some(id)
    }

    fn read_artifact(&self, project: &str, slug: &str, artifact: &str) -> Result<String> {
        let m = self.manifest(project)?;
        let path = artifact_path(slug, artifact)?;
        let argv = read_argv(&m.sandbox_container(), &path);
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let (status, out) = proc::run_capture("docker", &args)?;
        proc::require_success("reading the artifact out of the sandbox", status)?;
        audit::log_exec(project, &m, "studio", &argv, status.code())?;
        Ok(out)
    }

    fn write_artifact(
        &self,
        project: &str,
        slug: &str,
        artifact: &str,
        local: &Path,
    ) -> Result<()> {
        let m = self.manifest(project)?;
        let path = artifact_path(slug, artifact)?;
        let argv = write_back_argv(&m.sandbox_container(), &path);
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let status = proc::run_with_stdin_file("docker", &args, local)?;
        audit::log_exec(project, &m, "studio", &argv, status.code())?;
        proc::require_success("writing the artifact back into the sandbox", status)
    }

    fn approve(&self, project: &str, command: &[String]) -> Result<String> {
        // Only ever keel's own `approve` command, taken verbatim from
        // `keel next --json`. Anything else is a bug, not a thing to run.
        if command.first().map(String::as_str) != Some("keel")
            || command.get(1).map(String::as_str) != Some("approve")
        {
            anyhow::bail!("refusing to run '{}' as an approval", command.join(" "));
        }
        let (_, out) = self.exec(project, command)?;
        Ok(out)
    }
}

/// A turn coming back from its own thread. Keypresses are read straight
/// from the terminal, so this channel carries only completions.
pub enum Event {
    TurnDone {
        project: String,
        text: String,
        failed: bool,
    },
}

/// Send a turn without waiting for it.
///
/// A turn is a whole agent invocation — seconds to minutes. Running it on
/// the event-loop thread would freeze the console for its duration, so it
/// goes to its own thread and reports back through the channel. The caller
/// marks the project pending and keeps reading keys.
pub fn spawn_turn<F>(tx: &Sender<Event>, project: String, work: F)
where
    F: FnOnce() -> Result<(String, bool)> + Send + 'static,
{
    let tx = tx.clone();
    std::thread::spawn(move || {
        let (text, failed) = match work() {
            Ok(pair) => pair,
            Err(e) => (format!("{e:?}"), true),
        };
        // A closed channel just means the console has already exited.
        let _ = tx.send(Event::TurnDone {
            project,
            text,
            failed,
        });
    });
}

/// Read an artifact out of the container, hand it to `$EDITOR` on the
/// host, and write it back the way it came. Returns whether anything
/// changed — an unmodified file is not written back, so opening one to
/// read it leaves no trace in the sandbox.
pub fn edit_artifact(
    host: &dyn Host,
    project: &str,
    slug: &str,
    artifact: &str,
    open: &dyn Fn(&Path) -> Result<()>,
) -> Result<bool> {
    let before = host.read_artifact(project, slug, artifact)?;
    let local = std::env::temp_dir().join(format!(
        "moor-studio-{project}-{slug}-{artifact}-{}.md",
        std::process::id()
    ));
    std::fs::write(&local, &before)
        .with_context(|| format!("staging {} for editing", local.display()))?;
    open(&local)?;
    let after = std::fs::read_to_string(&local)
        .with_context(|| format!("reading {} back", local.display()))?;
    let changed = after != before;
    if changed {
        host.write_artifact(project, slug, artifact, &local)?;
    }
    let _ = std::fs::remove_file(&local);
    Ok(changed)
}

fn open_in_editor(path: &Path) -> Result<()> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let path = path.to_string_lossy().to_string();
    let status = proc::run_inherit(&editor, &[&path])?;
    proc::require_success(&format!("`{editor}`"), status)
}

fn translate(key: crossterm::event::KeyEvent) -> Option<Key> {
    use crossterm::event::KeyCode;
    match key.code {
        KeyCode::Char(c) => Some(Key::Char(c)),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Esc => Some(Key::Esc),
        _ => None,
    }
}

/// Restores the terminal however the loop ends — clean exit, `?`, or
/// panic. Without this a crash leaves the operator in raw mode.
struct RawMode;

impl RawMode {
    fn enter() -> Result<RawMode> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(RawMode)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let mut out = std::io::stdout();
        let _ = crossterm::execute!(out, crossterm::cursor::Show);
        let _ = writeln!(out);
    }
}

fn draw(console: &Console) -> Result<()> {
    let (w, h) = crossterm::terminal::size().unwrap_or((100, 30));
    let mut out = std::io::stdout();
    crossterm::execute!(
        out,
        crossterm::cursor::Hide,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0),
    )?;
    // Raw mode means a bare "\n" does not return the cursor to column 0.
    for line in render::frame(console, w as usize, h as usize).lines() {
        write!(out, "{line}\r\n")?;
    }
    out.flush()?;
    Ok(())
}

pub fn run(names: Vec<String>) -> Result<()> {
    if names.is_empty() {
        anyhow::bail!("no projects yet — try `moor new <name>`");
    }
    let host = Docker;
    let mut console = Console::new(&names);
    console.refresh(&host)?;

    let _raw = RawMode::enter()?;
    let (tx, rx) = mpsc::channel();
    loop {
        drain(&mut console, &rx);
        draw(&console)?;
        if !crossterm::event::poll(Duration::from_millis(120))? {
            continue;
        }
        let crossterm::event::Event::Key(key) = crossterm::event::read()? else {
            continue;
        };
        if key.kind != crossterm::event::KeyEventKind::Press {
            continue;
        }
        let Some(key) = translate(key) else { continue };
        match console.handle_key(key) {
            Action::None => {}
            Action::Quit => return Ok(()),
            Action::Refresh => console.refresh(&host)?,
            Action::SendTurn {
                project,
                prompt,
                resume,
            } => {
                let p = project.clone();
                // Chained and transcribed by `session::run_turn` — the
                // same path `moor ask` uses. Nothing here logs separately.
                spawn_turn(&tx, project, move || {
                    let outcome =
                        session::run_turn(&p, session::Role::Build, &prompt, resume.is_none())?;
                    Ok((outcome.text, outcome.failed))
                });
            }
            Action::Approve {
                project,
                slug,
                command,
            } => {
                let out = host.approve(&project, &command)?;
                console.note(&project, &format!("approved '{slug}': {}", out.trim()));
                console.refresh(&host)?;
            }
            Action::EditArtifact {
                project,
                slug,
                artifact,
            } => {
                // $EDITOR owns the terminal while it runs.
                crossterm::terminal::disable_raw_mode()?;
                let result = edit_artifact(&host, &project, &slug, &artifact, &open_in_editor);
                crossterm::terminal::enable_raw_mode()?;
                match result {
                    Ok(true) => console.note(&project, &format!("wrote {artifact}.md back")),
                    Ok(false) => console.note(&project, &format!("{artifact}.md unchanged")),
                    Err(e) => console.status = format!("edit failed: {e}"),
                }
            }
        }
    }
}

/// Fold every turn that has come back since the last pass into the
/// console. Non-blocking: a turn still in flight simply isn't here yet.
fn drain(console: &mut Console, rx: &Receiver<Event>) {
    while let Ok(Event::TurnDone {
        project,
        text,
        failed,
    }) = rx.try_recv()
    {
        console.complete_turn(&project, &text, failed);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// A `Host` that records what it was asked for. The console's own
    /// behaviour is what's under test here — how many container queries a
    /// refresh costs, what the write-back carries — so the boundary is
    /// where the fake goes, not the logic.
    pub struct FakeHost {
        container_queries: AtomicUsize,
        keel_next_calls: AtomicUsize,
        last_query: Mutex<Vec<String>>,
        running: Vec<String>,
        next_json: String,
        sessions: Vec<(String, String)>,
        artifacts: Mutex<Vec<(String, String, String, String)>>,
        pub writes: Mutex<Vec<(String, String, String, String)>>,
        pub approvals: Mutex<Vec<Vec<String>>>,
    }

    impl FakeHost {
        pub fn new() -> FakeHost {
            FakeHost {
                container_queries: AtomicUsize::new(0),
                keel_next_calls: AtomicUsize::new(0),
                last_query: Mutex::new(vec![]),
                running: vec![],
                next_json: r#"{"specs":[]}"#.to_string(),
                sessions: vec![],
                artifacts: Mutex::new(vec![]),
                writes: Mutex::new(vec![]),
                approvals: Mutex::new(vec![]),
            }
        }

        pub fn with_running(mut self, names: &[&str]) -> Self {
            self.running = names.iter().map(|s| s.to_string()).collect();
            self
        }

        pub fn with_next(mut self, json: &str) -> Self {
            self.next_json = json.to_string();
            self
        }

        pub fn with_artifact(self, project: &str, slug: &str, artifact: &str, body: &str) -> Self {
            self.artifacts.lock().unwrap().push((
                project.into(),
                slug.into(),
                artifact.into(),
                body.into(),
            ));
            self
        }

        pub fn container_queries(&self) -> usize {
            self.container_queries.load(Ordering::SeqCst)
        }

        pub fn keel_next_calls(&self) -> usize {
            self.keel_next_calls.load(Ordering::SeqCst)
        }

        pub fn last_query_names(&self) -> Vec<String> {
            self.last_query.lock().unwrap().clone()
        }
    }

    impl Host for FakeHost {
        fn running_projects(&self, names: &[String]) -> Result<Vec<String>> {
            self.container_queries.fetch_add(1, Ordering::SeqCst);
            *self.last_query.lock().unwrap() = names.to_vec();
            Ok(self.running.clone())
        }

        fn keel_next(&self, _project: &str) -> Result<String> {
            self.keel_next_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.next_json.clone())
        }

        fn stored_session(&self, project: &str) -> Option<String> {
            self.sessions
                .iter()
                .find(|(p, _)| p == project)
                .map(|(_, id)| id.clone())
        }

        fn read_artifact(&self, project: &str, slug: &str, artifact: &str) -> Result<String> {
            self.artifacts
                .lock()
                .unwrap()
                .iter()
                .find(|(p, s, a, _)| p == project && s == slug && a == artifact)
                .map(|(_, _, _, body)| body.clone())
                .ok_or_else(|| anyhow::anyhow!("no such artifact"))
        }

        fn write_artifact(
            &self,
            project: &str,
            slug: &str,
            artifact: &str,
            local: &Path,
        ) -> Result<()> {
            let body = std::fs::read_to_string(local)?;
            self.writes
                .lock()
                .unwrap()
                .push((project.into(), slug.into(), artifact.into(), body));
            Ok(())
        }

        fn approve(&self, _project: &str, command: &[String]) -> Result<String> {
            self.approvals.lock().unwrap().push(command.to_vec());
            Ok("approved".into())
        }
    }

    /// AC-2
    #[test]
    fn input_is_handled_while_turn_is_pending() {
        let mut console = Console::new(&["alpha".to_string(), "beta".to_string()]);
        for p in &mut console.projects {
            p.up = true;
        }
        let (tx, rx) = mpsc::channel();

        // A turn that will not return until this test lets it, standing in
        // for a real agent invocation taking seconds to minutes.
        let (release, hold) = mpsc::channel::<()>();
        let started = std::time::Instant::now();
        spawn_turn(&tx, "alpha".to_string(), move || {
            hold.recv().ok();
            Ok(("the answer".to_string(), false))
        });
        console.begin_turn("alpha");
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "dispatching a turn must not wait for the child process"
        );

        // With that turn still in flight: the project is marked awaiting,
        // and keys keep working.
        assert!(console.projects[0].pending());
        assert_eq!(console.handle_key(Key::Down), Action::None);
        assert_eq!(console.selected, 1);
        for ch in "meanwhile".chars() {
            console.handle_key(Key::Char(ch));
        }
        assert_eq!(console.projects[1].input, "meanwhile");
        // Including a turn for the *other* project, which is not blocked by
        // alpha's.
        assert!(matches!(
            console.handle_key(Key::Enter),
            Action::SendTurn { .. }
        ));
        // Nothing has come back yet, so nothing has been drawn for alpha.
        drain(&mut console, &rx);
        assert!(console.projects[0].pending());
        assert!(console.projects[0].transcript.is_empty());

        // A second turn for a project already awaiting one is refused
        // rather than queued.
        console.selected = 0;
        for ch in "again".chars() {
            console.handle_key(Key::Char(ch));
        }
        assert_eq!(console.handle_key(Key::Enter), Action::None);
        assert!(console.status.contains("awaiting a response"));

        // Let it finish: the reply arrives over the channel and clears the
        // marker.
        release.send(()).unwrap();
        let Event::TurnDone {
            project,
            text,
            failed,
        } = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(project, "alpha");
        console.complete_turn(&project, &text, failed);
        assert!(!console.projects[0].pending());
        assert_eq!(
            console.projects[0].transcript.last(),
            Some(&state::Line::Agent("the answer".into()))
        );
    }

    /// AC-5
    #[test]
    fn artifact_edit_writes_back_over_exec() {
        let host = FakeHost::new().with_artifact(
            "demo",
            "blast-radius",
            "plan",
            "# Plan\n\noriginal body\n",
        );

        // The "editor" is whatever $EDITOR would have done to the file.
        let edited = edit_artifact(&host, "demo", "blast-radius", "plan", &|path| {
            let body = std::fs::read_to_string(path)?;
            assert!(body.contains("original body"), "the real file was staged");
            std::fs::write(
                path,
                body.replace("original body", "body the operator fixed"),
            )?;
            Ok(())
        })
        .unwrap();

        assert!(edited);
        let writes = host.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        let (project, slug, artifact, body) = &writes[0];
        assert_eq!(
            (project.as_str(), slug.as_str(), artifact.as_str()),
            ("demo", "blast-radius", "plan")
        );
        assert!(body.contains("body the operator fixed"));
        assert!(body.starts_with("# Plan"));
        drop(writes);

        // Opening without changing anything writes nothing back.
        let host2 = FakeHost::new().with_artifact("demo", "blast-radius", "spec", "unchanged\n");
        assert!(!edit_artifact(&host2, "demo", "blast-radius", "spec", &|_| Ok(())).unwrap());
        assert!(host2.writes.lock().unwrap().is_empty());

        // The write-back goes through `docker exec -i ... sh -c 'cat > …'`
        // — stdin, the same mechanism `proc::run_with_stdin_file` uses. No
        // bind mount is involved, and the argv proves it.
        let path = artifact_path("blast-radius", "plan").unwrap();
        assert_eq!(path, "/workspace/.keel/specs/blast-radius/plan.md");
        let argv = write_back_argv("demo-sandbox", &path);
        assert_eq!(
            argv,
            vec![
                "exec",
                "-i",
                "demo-sandbox",
                "sh",
                "-c",
                "cat > /workspace/.keel/specs/blast-radius/plan.md",
            ]
        );
        for argv in [
            write_back_argv("demo-sandbox", &path),
            read_argv("demo-sandbox", &path),
        ] {
            for flag in ["-v", "--volume", "--mount", "-w", "--privileged"] {
                assert!(!argv.iter().any(|a| a == flag), "{flag} in {argv:?}");
            }
            assert!(
                !argv.iter().any(|a| a.contains(":/")),
                "host path in {argv:?}"
            );
        }

        // A slug that isn't one never reaches a path at all.
        assert!(artifact_path("../../etc", "plan").is_err());
        assert!(artifact_path("ok", "passwd").is_err());
    }

    /// AC-8
    #[test]
    fn console_writes_no_second_log() {
        // Functionally: every command the console issues goes out through
        // the host, which records it on the existing chain — turns via
        // `session::run_turn`, keel verbs via `audit::log_exec`.
        let host = FakeHost::new()
            .with_running(&["demo"])
            .with_next(r#"{"specs":[{"slug":"s","stage":"plan-approval","command":"keel approve s --stage plan","complete":false}]}"#);
        let mut console = Console::new(&["demo".to_string()]);
        console.refresh(&host).unwrap();
        console.handle_key(Key::Char('a'));
        let action = console.handle_key(Key::Char('y'));
        let Action::Approve {
            project, command, ..
        } = action
        else {
            panic!("expected an approval, got {action:?}");
        };
        host.approve(&project, &command).unwrap();
        let approvals = host.approvals.lock().unwrap();
        assert_eq!(approvals.len(), 1);
        // Verbatim from keel's own `next --json`, not reconstructed.
        assert_eq!(
            approvals[0],
            vec!["keel", "approve", "s", "--stage", "plan"]
        );
        drop(approvals);

        // Structurally: no studio source opens a log of its own. The scan
        // stops at each file's own test module, or it would match the
        // literals in this very assertion.
        let sources = [
            ("studio/mod.rs", include_str!("mod.rs")),
            ("studio/state.rs", include_str!("state.rs")),
            ("studio/render.rs", include_str!("render.rs")),
            (
                "commands/studio_cmd.rs",
                include_str!("../commands/studio_cmd.rs"),
            ),
        ];
        for (name, src) in sources {
            let code = src.split("#[cfg(test)]").next().unwrap();
            for writer in [
                "append_chained",
                "chain_log_path",
                "transcript_path",
                "OpenOptions",
                "audit_dir",
                ".log",
            ] {
                assert!(
                    !code.contains(writer),
                    "{name} reaches for '{writer}' — the chain is the only record"
                );
            }
        }
        // And it does route through the two writers that already exist.
        let code = include_str!("mod.rs").split("#[cfg(test)]").next().unwrap();
        assert!(code.contains("audit::log_exec"));
        assert!(code.contains("session::run_turn"));
    }

    #[test]
    fn approve_refuses_anything_that_is_not_keels_own_approve() {
        let docker = Docker;
        for argv in [
            vec!["keel".to_string(), "run".to_string(), "s".to_string()],
            vec!["rm".to_string(), "-rf".to_string(), "/".to_string()],
            vec!["keel".to_string()],
            vec![],
        ] {
            assert!(
                docker.approve("demo", &argv).is_err(),
                "{argv:?} must not run as an approval"
            );
        }
    }

    #[test]
    fn a_turn_that_fails_is_reported_as_a_failed_turn() {
        let (tx, rx) = mpsc::channel();
        spawn_turn(&tx, "demo".to_string(), || {
            anyhow::bail!("the sandbox is not running")
        });
        let Event::TurnDone { failed, text, .. } = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(failed);
        assert!(text.contains("not running"));
    }
}
