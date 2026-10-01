mod audit;
mod canary;
mod commands;
mod compose;
mod egress_log;
mod guide;
mod manifest;
mod paths;
mod posture;
mod proc;
mod secrets;
mod session;
mod studio;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

// `version` with no value makes clap source `moor --version` / `moor -V`
// from CARGO_PKG_VERSION (the version in cli/Cargo.toml), so there is no
// second place to bump — it always matches the released crate.
#[derive(Parser)]
#[command(
    name = "moor",
    version,
    about = "Build software with an AI agent inside a locked-down container",
    after_help = "Not sure what to do next? Run `moor next`."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Ordered by how a project is worked: set it up, follow the guided loop,
/// then everything else. `moor --help` lists them in this order.
#[derive(Subcommand)]
enum Command {
    /// Create a new project with its own sandbox.
    ///
    /// Writes the project's manifest, starts its sandbox and egress
    /// containers, optionally creates a private GitHub repo, and prepares
    /// the workspace for specs.
    New {
        name: String,
        /// moor/base, moor/node, moor/rust, or moor/python
        #[arg(long, default_value = "moor/base:latest")]
        image: String,
        /// Also create a private GitHub repo via `gh repo create` (asks
        /// for confirmation before doing anything).
        #[arg(long)]
        github: bool,
    },
    /// Bring an existing local git repo into a new sandboxed project.
    ///
    /// Transfers its full history via a `git bundle`: the source directory
    /// is never bind-mounted, only a one-shot bundle file crosses into the
    /// container. Preserves an existing GitHub remote if there is one.
    Import {
        name: String,
        /// Path to the existing local repo to import.
        #[arg(long, value_name = "PATH")]
        from: PathBuf,
        /// moor/base, moor/node, moor/rust, or moor/python
        /// — auto-detected from the source repo (Cargo.toml, package.json,
        /// pyproject.toml/requirements.txt) if not given.
        #[arg(long)]
        image: Option<String>,
        /// If the source repo has no GitHub remote, create one (asks for
        /// confirmation). Ignored if it already has one.
        #[arg(long)]
        github: bool,
    },
    /// Start (or restart) a project's sandbox.
    Up { name: String },
    /// Stop a project's sandbox.
    Down { name: String },
    /// Write a spec on this Mac, then send it into the sandbox.
    #[command(subcommand)]
    Spec(SpecCommand),
    /// Show where the active spec stands and the one command that moves it on.
    ///
    /// Every command it prints runs as shown. Project defaults to the one
    /// set by `moor use`, else the one sandbox that's up, else the only
    /// project you have.
    Next {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// List every spec and its step instead.
        #[arg(long)]
        all: bool,
    },
    /// Take the active spec's next steps until something needs you.
    ///
    /// Runs the checks, drafts the plan, and builds, one step after
    /// another, stopping at the first approval, failure, or step that
    /// didn't move the spec on.
    Go {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// Check the build step without the agent: re-check the work already
        /// in the sandbox, after fixing it by hand or with `moor ask`.
        #[arg(long)]
        check: bool,
    },
    /// Approve what the active spec is waiting on (shows it first, then asks).
    Approve {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// Approve without asking (for when there's no terminal).
        #[arg(long)]
        yes: bool,
    },
    /// Reject what the active spec is waiting on, saying why.
    Reject {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// Why, so the next attempt can address it.
        why: String,
    },
    /// Ship an approved spec: commit it on its own branch, push it, open a PR.
    ///
    /// Shows exactly which files it will commit (the spec's change, its
    /// folder and its runs, nothing of other specs) and asks first. Returns
    /// the sandbox to its trunk branch afterwards.
    Ship {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// The spec to ship, if more than one is waiting.
        spec: Option<String>,
        /// Ship without asking (for when there's no terminal).
        #[arg(long)]
        yes: bool,
    },
    /// Set the default project, and optionally pin its active spec.
    Use {
        name: String,
        /// Also pin the spec `moor next` guides this project through.
        #[arg(long, value_name = "SLUG")]
        spec: Option<String>,
    },
    /// Print a spec, its plan or tasks, or the report of its checks.
    View {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// The spec's name, e.g. `blast-radius`.
        slug: String,
        /// spec | plan | tasks | report (every run's checks) | diff (the change)
        artifact: String,
    },
    /// Ask the project's agent something, or have it make a change.
    ///
    /// A single turn, recorded in the audit trail and resumable. Unlike
    /// `moor shell`, the whole exchange is recorded; unlike `moor recipe`,
    /// you can think the change through first.
    Ask {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// brainstorm (read-only) | build (can edit files and run the
        /// project's checks).
        #[arg(long, default_value = "brainstorm")]
        role: String,
        /// Start a new session instead of resuming the stored one.
        #[arg(long)]
        new: bool,
        /// Write the agent's answer out as a recipe file, but only if it
        /// parses — see `moor recipe`.
        #[arg(long, value_name = "PATH")]
        emit_recipe: Option<PathBuf>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        prompt: Vec<String>,
    },
    /// Take a change from a written description all the way to a build.
    ///
    /// Drives spec, checks, plan, and build from a loosely described
    /// recipe file, stopping whenever it needs your approval or a check
    /// still fails after retrying. Re-run the same command to continue.
    /// See docs/decisions/0005-recipe.md.
    Recipe {
        name: String,
        /// Path to the recipe file (YAML front matter — slug, scope —
        /// followed by a free-text description of the desired outcome).
        file: PathBuf,
    },
    /// Show (or follow) a running `moor recipe`'s progress.
    ///
    /// Stage changes, check attempts, and pauses for approval, read from
    /// the audit trail, so you can watch from another terminal. For full
    /// command output, see `moor audit`.
    Logs {
        name: String,
        /// Keep watching for new events instead of exiting after printing
        /// what's there so far.
        #[arg(short, long)]
        follow: bool,
        /// How many of the most recent events to print before following.
        #[arg(short = 'n', long, default_value_t = 20)]
        lines: usize,
    },
    /// A console over every project: status, stages, the agent, approvals.
    ///
    /// Local terminal only — no socket, no port, no daemon.
    Studio {
        /// Show just this project instead of every project on disk.
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
    },
    /// Open an interactive shell inside a project's sandbox.
    Shell { name: String },
    /// Run one command inside a project's sandbox.
    Run {
        name: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        cmd: Vec<String>,
    },
    /// Refresh the sandbox's current branch from its GitHub remote.
    ///
    /// Pulls over HTTPS with a token injected inside the container, so it
    /// works even though the sandbox has no SSH client and never persists
    /// a credential. See docs/decisions/0010-sandbox-git-transport.md.
    Pull {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
    },
    /// List all projects and whether their sandboxes are up.
    Status,
    /// Show a project's tamper-evident audit trail.
    ///
    /// Folds in new egress log entries first.
    Audit {
        name: String,
        /// Recompute the hash chain and report whether it's intact instead
        /// of printing the trail.
        #[arg(long)]
        verify: bool,
        /// Export the chain plus the latest evidence bundle as a tar.gz
        /// into this directory instead of printing the trail.
        #[arg(long, value_name = "DIR")]
        export: Option<PathBuf>,
    },
    /// Package a run's evidence with this project's audit trail, and verify it.
    ///
    /// Runs in throwaway containers with no network, never the sandbox.
    Bundle {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// The run to bundle; defaults to the latest.
        run: Option<String>,
        /// Directory to write the bundle into (default: current directory)
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Check a running project's sandbox is locked down the way it should be.
    ///
    /// Verifies every hardening control (read-only rootfs, dropped
    /// capabilities, no bind mounts, non-root user, no default-bridge
    /// network, ...) and runs the active breakout battery (canary domain,
    /// read-only fs, docker.sock).
    Selftest { name: String },
    /// Manage a project's secrets in the macOS Keychain.
    ///
    /// An alternative to exporting them into your shell before every
    /// `moor up`/`new`.
    #[command(subcommand)]
    Secrets(SecretsCommand),
    /// Pass arguments straight to the workflow engine inside the sandbox.
    ///
    /// The escape hatch under `moor next`/`go`/`approve`/`reject`, for
    /// anything they don't cover. See docs/ARCHITECTURE.md.
    #[command(hide = true)]
    Keel {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum SpecCommand {
    /// Write a starter spec file here, with the rules it must meet at the top.
    ///
    /// Nothing is sent anywhere: write the spec by hand or with your AI
    /// assistant, then `moor spec push` it.
    New {
        /// The spec's short name, e.g. `login-endpoint`.
        name: String,
        /// A human-readable title (defaults to the name).
        #[arg(long)]
        title: Option<String>,
        /// Paths the change may touch, as globs (repeatable; default `src/**`).
        #[arg(long)]
        scope: Vec<String>,
    },
    /// Send a spec file into the sandbox, make it the active spec, and check it.
    Push {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// The spec file, e.g. `login-endpoint.spec.md`.
        file: PathBuf,
    },
}

#[derive(Subcommand)]
enum SecretsCommand {
    /// Store a secret's value in the Keychain (prompts for it, hidden
    /// where the terminal supports it). Overwrites any existing value.
    Set { project: String, name: String },
    /// Remove a secret from the Keychain.
    Unset { project: String, name: String },
    /// Show where each of the project's declared secrets (moor.yaml's
    /// `secrets:` list) would currently be resolved from: your shell's
    /// environment, the Keychain, or neither.
    Status { project: String },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::New {
            name,
            image,
            github,
        } => commands::new_cmd::run(&name, &image, github),
        Command::Import {
            name,
            from,
            image,
            github,
        } => commands::import_cmd::run(&name, &from, image, github),
        Command::Up { name } => commands::up::run(&name),
        Command::Down { name } => commands::down::run(&name),
        Command::Shell { name } => commands::shell::run(&name),
        Command::Run { name, cmd } => commands::run_cmd::run(&name, &cmd),
        Command::Pull { project } => commands::pull_cmd::run(project),
        Command::Keel { project, args } => commands::resolve_project_announced(project)
            .and_then(|name| commands::keel_cmd::run(&name, &args)),
        Command::Bundle { project, run, out } => commands::resolve_project_announced(project)
            .and_then(|name| commands::bundle_cmd::run(&name, run.as_deref(), out)),
        Command::View {
            project,
            slug,
            artifact,
        } => commands::resolve_project_announced(project)
            .and_then(|name| commands::view::run(&name, &slug, &artifact)),
        Command::Spec(SpecCommand::New { name, title, scope }) => {
            commands::spec_cmd::new(&name, title, &scope)
        }
        Command::Spec(SpecCommand::Push { project, file }) => {
            commands::spec_cmd::push(project, &file)
        }
        Command::Next { project, all } => commands::next_cmd::run(project, all),
        Command::Approve { project, yes } => commands::flow_cmd::approve(project, yes),
        Command::Reject { project, why } => commands::flow_cmd::reject(project, &why),
        Command::Go { project, check } => commands::flow_cmd::go(project, check),
        Command::Ship { project, spec, yes } => commands::ship_cmd::run(project, spec, yes),
        Command::Use { name, spec } => commands::use_cmd::run(&name, spec.as_deref()),
        Command::Status => commands::status::run(),
        Command::Audit {
            name,
            verify,
            export,
        } => commands::audit_cmd::run(&name, verify, export),
        Command::Selftest { name } => commands::selftest::run(&name),
        Command::Secrets(SecretsCommand::Set { project, name }) => secrets::set(&project, &name),
        Command::Secrets(SecretsCommand::Unset { project, name }) => {
            secrets::unset(&project, &name)
        }
        Command::Secrets(SecretsCommand::Status { project }) => {
            commands::secrets_status::run(&project)
        }
        Command::Ask {
            project,
            role,
            new,
            emit_recipe,
            prompt,
        } => commands::resolve_project_announced(project).and_then(|name| {
            commands::ask_cmd::run(&name, &role, new, emit_recipe.as_deref(), &prompt)
        }),
        Command::Studio { project } => commands::studio_cmd::run(project),
        Command::Recipe { name, file } => commands::recipe::run(&name, &file),
        Command::Logs {
            name,
            follow,
            lines,
        } => commands::logs::run(&name, follow, lines),
    };

    if let Err(e) = result {
        eprintln!("error: {e:?}");
        std::process::exit(1);
    }
}
