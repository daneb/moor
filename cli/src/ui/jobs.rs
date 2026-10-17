//! Long-running work the console starts: every action is the `moor`
//! binary itself, run as a child with its output captured line by line.
//! That is deliberate — `moor go` from the console is byte-for-byte `moor
//! go` from a terminal, with the same checks, the same prompts answered
//! the same way (`--yes` only where the operator confirmed in the page),
//! and the same audit entries. The console adds a view, not a second
//! implementation.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Output lines kept per job. A build's output is long but not unbounded;
/// past this, the oldest lines go — the full record is in the audit chain
/// and keel's own evidence, not here.
const MAX_LINES: usize = 20_000;

#[derive(Default)]
struct Inner {
    lines: Vec<String>,
    /// How many lines were dropped from the front to stay under MAX_LINES.
    dropped: usize,
    done: bool,
    exit_code: Option<i32>,
    finished: Option<String>,
    pid: Option<u32>,
    error: Option<String>,
}

pub struct Job {
    pub id: u64,
    pub kind: String,
    pub label: String,
    pub project: Option<String>,
    /// The argv as shown to the operator. Secrets never travel in argv
    /// (they go through stdin), so this is safe to display.
    pub argv: Vec<String>,
    pub started: String,
    inner: Mutex<Inner>,
    changed: Condvar,
}

impl Job {
    fn push(&self, line: String) {
        let mut g = self.inner.lock().unwrap();
        g.lines.push(crate::studio::render::sanitize(&line));
        if g.lines.len() > MAX_LINES {
            let excess = g.lines.len() - MAX_LINES;
            g.lines.drain(..excess);
            g.dropped += excess;
        }
        self.changed.notify_all();
    }

    fn finish(&self, code: Option<i32>, error: Option<String>) {
        let mut g = self.inner.lock().unwrap();
        g.done = true;
        g.exit_code = code;
        g.error = error;
        g.finished = Some(now());
        self.changed.notify_all();
    }

    /// Everything the page needs to draw this job, with the lines from
    /// absolute index `after` on.
    pub fn snapshot(&self, after: usize) -> Value {
        let g = self.inner.lock().unwrap();
        let start = after.saturating_sub(g.dropped).min(g.lines.len());
        json!({
            "id": self.id,
            "kind": self.kind,
            "label": self.label,
            "project": self.project,
            "argv": self.argv,
            "started": self.started,
            "finished": g.finished,
            "done": g.done,
            "exit_code": g.exit_code,
            "error": g.error,
            "lines": g.lines[start..],
            "next": g.dropped + g.lines.len(),
        })
    }

    pub fn summary(&self) -> Value {
        let g = self.inner.lock().unwrap();
        json!({
            "id": self.id,
            "kind": self.kind,
            "label": self.label,
            "project": self.project,
            "started": self.started,
            "finished": g.finished,
            "done": g.done,
            "exit_code": g.exit_code,
            "error": g.error,
            "line_count": g.dropped + g.lines.len(),
        })
    }

    pub fn is_done(&self) -> bool {
        self.inner.lock().unwrap().done
    }

    /// Block until there is something past `after` to send, the job ends,
    /// or `timeout` passes. Returns (new lines, next index, done).
    pub fn wait(&self, after: usize, timeout: Duration) -> (Vec<String>, usize, bool) {
        let g = self.inner.lock().unwrap();
        let has_new = |g: &Inner| g.dropped + g.lines.len() > after || g.done;
        let (g, _) = self
            .changed
            .wait_timeout_while(g, timeout, |g| !has_new(g))
            .unwrap();
        let start = after.saturating_sub(g.dropped).min(g.lines.len());
        (g.lines[start..].to_vec(), g.dropped + g.lines.len(), g.done)
    }

    /// Ask the child to stop. SIGTERM, so `moor` can leave the audit
    /// entry it was about to write; nothing here force-kills.
    pub fn cancel(&self) -> Result<()> {
        let pid = self.inner.lock().unwrap().pid;
        let Some(pid) = pid else {
            anyhow::bail!("this job has no running process");
        };
        let status = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()?;
        if !status.success() {
            anyhow::bail!("could not signal process {pid}");
        }
        Ok(())
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Default)]
pub struct Jobs {
    next_id: AtomicU64,
    all: Mutex<Vec<Arc<Job>>>,
}

impl Jobs {
    pub fn list(&self) -> Vec<Arc<Job>> {
        self.all.lock().unwrap().clone()
    }

    pub fn get(&self, id: u64) -> Option<Arc<Job>> {
        self.all
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.id == id)
            .cloned()
    }

    /// Whether a job of this kind is still running for this project —
    /// two `moor go`s on one sandbox would race each other.
    pub fn busy(&self, project: &str) -> Option<Arc<Job>> {
        self.all
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.project.as_deref() == Some(project) && !j.is_done() && j.exclusive())
            .cloned()
    }

    /// Start `moor <argv>` as a child of this process, with `stdin` (if
    /// any) written to it and closed. Returns at once; the job fills in
    /// from its reader threads.
    pub fn spawn(
        &self,
        kind: &str,
        label: &str,
        project: Option<&str>,
        argv: Vec<String>,
        stdin: Option<String>,
    ) -> Result<Arc<Job>> {
        let exe = std::env::current_exe().context("locating the moor binary")?;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let job = Arc::new(Job {
            id,
            kind: kind.to_string(),
            label: label.to_string(),
            project: project.map(str::to_string),
            argv: argv.clone(),
            started: now(),
            inner: Mutex::new(Inner::default()),
            changed: Condvar::new(),
        });
        self.all.lock().unwrap().push(job.clone());

        let mut child = Command::new(&exe)
            .args(&argv)
            .env("MOOR_UI", "1")
            .env("NO_COLOR", "1")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting moor {}", argv.join(" ")))?;
        job.inner.lock().unwrap().pid = Some(child.id());

        if let (Some(mut input), Some(text)) = (child.stdin.take(), stdin) {
            // Written on its own thread so a child that never reads stdin
            // cannot stall the spawn.
            std::thread::spawn(move || {
                let _ = input.write_all(text.as_bytes());
                let _ = input.flush();
            });
        }

        let stdout = child.stdout.take().context("capturing stdout")?;
        let stderr = child.stderr.take().context("capturing stderr")?;
        let readers = [
            spawn_reader(job.clone(), Box::new(stdout)),
            spawn_reader(job.clone(), Box::new(stderr)),
        ];
        let waiter = job.clone();
        std::thread::spawn(move || {
            let status = child.wait();
            for r in readers {
                let _ = r.join();
            }
            match status {
                Ok(s) => waiter.finish(s.code(), None),
                Err(e) => waiter.finish(None, Some(e.to_string())),
            }
        });
        Ok(job)
    }
}

impl Job {
    /// Jobs that must not overlap on one sandbox. Reads and the agent's
    /// chat can interleave; the pipeline's own steps cannot.
    fn exclusive(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "go" | "approve" | "reject" | "ship" | "spec_push" | "recipe" | "up" | "down" | "gate"
        )
    }
}

fn spawn_reader(
    job: Arc<Job>,
    source: Box<dyn std::io::Read + Send>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let reader = BufReader::new(source);
        for line in reader.split(b'\n') {
            match line {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes)
                        .trim_end_matches('\r')
                        .to_string();
                    job.push(text);
                }
                Err(_) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(kind: &str) -> Job {
        Job {
            id: 1,
            kind: kind.into(),
            label: "t".into(),
            project: Some("p".into()),
            argv: vec![],
            started: now(),
            inner: Mutex::new(Inner::default()),
            changed: Condvar::new(),
        }
    }

    #[test]
    fn lines_are_sanitized_and_indexed_absolutely() {
        let j = job("go");
        j.push("plain".into());
        j.push("\x1b[31mred\x1b[0m\r".into());
        let snap = j.snapshot(0);
        assert_eq!(snap["lines"], json!(["plain", "red"]));
        assert_eq!(snap["next"], 2);
        let snap = j.snapshot(1);
        assert_eq!(snap["lines"], json!(["red"]));
    }

    #[test]
    fn wait_returns_at_once_when_lines_exist_and_when_done() {
        let j = job("go");
        j.push("a".into());
        let (lines, next, done) = j.wait(0, Duration::from_millis(10));
        assert_eq!(lines, vec!["a"]);
        assert_eq!(next, 1);
        assert!(!done);
        let (lines, _, done) = j.wait(1, Duration::from_millis(10));
        assert!(lines.is_empty() && !done, "timed out with nothing new");
        j.finish(Some(0), None);
        let (_, _, done) = j.wait(1, Duration::from_millis(10));
        assert!(done);
    }

    #[test]
    fn pipeline_steps_are_exclusive_but_reads_are_not() {
        assert!(job("go").exclusive());
        assert!(job("approve").exclusive());
        assert!(!job("ask").exclusive());
        assert!(!job("selftest").exclusive());
    }
}
