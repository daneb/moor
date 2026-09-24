use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

/// Run a command with stdio inherited (the operator sees output live,
/// e.g. `docker compose up`, `docker exec -it ... bash`). Returns the exit
/// status; callers decide whether a non-zero code is fatal.
pub fn run_inherit(program: &str, args: &[&str]) -> Result<ExitStatus> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("spawning `{program} {}`", args.join(" ")))
}

/// Run a command and capture stdout as a String (used for `docker inspect`,
/// `docker ps`, etc. where we need to parse the result).
pub fn run_capture(program: &str, args: &[&str]) -> Result<(ExitStatus, String)> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("spawning `{program} {}`", args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok((output.status, stdout))
}

/// Like `run_capture`, but keeps stdout and stderr as separate strings.
/// Needed when stdout must stay machine-parseable (`claude --output-format
/// json`) *and* stderr is the only place the real explanation goes: a
/// `claude` that refuses to start writes nothing to stdout at all, so
/// discarding stderr turns "Invalid MCP configuration" into an
/// uninterpretable "EOF while parsing a value at line 1 column 0".
pub fn run_capture_split(program: &str, args: &[&str]) -> Result<(ExitStatus, String, String)> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("spawning `{program} {}`", args.join(" ")))?;
    Ok((
        output.status,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

/// Like `run_capture`, but returns stdout and stderr concatenated
/// (stdout first). Needed by anything that has to parse *or*
/// pattern-match a command's own explanatory text regardless of which
/// stream it chose: keel, for one, puts gate results on stdout but a
/// hard refusal like "already exists" on stderr — found by actually
/// separating the streams and comparing, not assumed from `2>&1` output,
/// which looks identical either way. Not chronologically interleaved
/// (stdout is read whole, then stderr), which is fine for pattern
/// matching and for handing the text to an agent, but not for a human
/// watching two genuinely concurrent streams in real time.
pub fn run_capture_combined(program: &str, args: &[&str]) -> Result<(ExitStatus, String)> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("spawning `{program} {}`", args.join(" ")))?;
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status, combined))
}

/// Run a command, streaming a local file's bytes to its stdin. Used to get
/// a file into a container whose root filesystem is read-only: `docker
/// cp`'s own copy mechanism needs write access it doesn't have there even
/// when the destination path is a writable tmpfs mount, but piping bytes
/// through `docker exec -i <container> sh -c 'cat > dest'`'s stdin only
/// ever touches that one writable destination.
pub fn run_with_stdin_file(program: &str, args: &[&str], stdin_path: &Path) -> Result<ExitStatus> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning `{program} {}`", args.join(" ")))?;

    let mut file = std::fs::File::open(stdin_path)
        .with_context(|| format!("opening {}", stdin_path.display()))?;
    // stdin is always Some right after spawn() with Stdio::piped(); taking
    // it and letting it drop after the copy closes the pipe, so the
    // child's `cat` sees EOF and exits.
    let mut stdin = child.stdin.take().expect("child stdin was piped");
    std::io::copy(&mut file, &mut stdin).context("streaming file to child stdin")?;
    drop(stdin);

    child.wait().context("waiting for child process")
}

pub fn require_success(what: &str, status: ExitStatus) -> Result<()> {
    if !status.success() {
        anyhow::bail!("{what} failed ({status})");
    }
    Ok(())
}
