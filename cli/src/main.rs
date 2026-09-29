mod audit;
mod canary;
mod commands;
mod compose;
mod egress_log;
mod manifest;
mod paths;
mod posture;
mod proc;
mod secrets;
mod session;
mod studio;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "moor", about = "Containerized, keel-driven AI sandboxes")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new project: manifest, sandbox+egress containers, optional
    /// GitHub repo, `keel init` inside the sandbox.
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
    /// Bring an existing local git repo (e.g. one you're already working
    /// on outside moor) into a new sandboxed project. Transfers its
    /// full history via a `git bundle` — the source directory is never
    /// bind-mounted, only a one-shot bundle file crosses into the
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
    /// Start (or restart) a project's sandbox + egress containers.
    Up { name: String },
    /// Stop a project's containers.
    Down { name: String },
    /// Open an interactive shell inside a project's sandbox.
    Shell { name: String },
    /// Run one command inside a project's sandbox (e.g. `keel run <spec>`).
    Run {
        name: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        cmd: Vec<String>,
    },
    /// Run `keel` inside a project's sandbox — sugar for
    /// `moor run <project> -- keel <args...>`. Project defaults to
    /// whatever `moor use` last set (or the one project you have, if
    /// there's only one); pass `--project` to override.
    Keel {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Build a keel evidence bundle that carries this project's host audit
    /// chain, then verify it with keel. Runs in throwaway containers with
    /// no network, never the sandbox. Project defaults the same way
    /// `moor keel` does.
    Bundle {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// keel run id; defaults to the latest run
        run: Option<String>,
        /// Directory to write the bundle into (default: current directory)
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Print one of keel's spec-produced markdown artifacts (spec.md,
    /// plan.md, tasks.md) for a given spec slug, lightly highlighted.
    /// Project defaults the same way `moor keel` does.
    View {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// The spec slug, e.g. `blast-radius`.
        slug: String,
        /// spec | plan | tasks
        artifact: String,
    },
    /// Set the default project `moor keel`/`moor view` (and their
    /// `--project`-taking siblings) use when it's omitted.
    Use { name: String },
    /// List all projects and their container status.
    Status,
    /// Show the host-side audit trail for a project (folds in new egress
    /// gateway log entries first).
    Audit {
        name: String,
        /// Recompute the hash chain and report whether it's intact instead
        /// of printing the trail.
        #[arg(long)]
        verify: bool,
        /// Export the chain plus the latest keel evidence bundle as a
        /// tar.gz into this directory instead of printing the trail.
        #[arg(long, value_name = "DIR")]
        export: Option<PathBuf>,
    },
    /// Verify a running project's container actually has every hardening
    /// control applied (read-only rootfs, dropped capabilities, no bind
    /// mounts, non-root user, no default-bridge network, ...) and run the
    /// active breakout battery (canary domain, read-only fs, docker.sock).
    Selftest { name: String },
    /// Manage a project's secrets in the macOS Keychain, as an
    /// alternative to exporting them into your shell before every
    /// `moor up`/`new`.
    #[command(subcommand)]
    Secrets(SecretsCommand),
    /// Drive keel's spec -> gate -> plan -> gate -> run -> gate pipeline
    /// from a loosely-described recipe file, stopping whenever a stage
    /// needs a human decision (spec/plan/merge approval, or a gate that
    /// still fails after retrying). Re-run the same command to continue
    /// once you've approved or fixed things by hand — see
    /// docs/decisions/0005-recipe.md.
    Recipe {
        name: String,
        /// Path to the recipe file (YAML front matter — slug, scope —
        /// followed by a free-text description of the desired outcome).
        file: PathBuf,
    },
    /// Put one instruction in front of the project's agent and get its
    /// answer back — a single chained, resumable turn. Unlike `moor
    /// shell`, the whole exchange lands in the audit chain; unlike `moor
    /// recipe`, you can think the change through first. Project defaults
    /// the same way `moor keel` does.
    Ask {
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
        /// brainstorm (Read/Glob/Grep only) | build (adds Edit/Write and
        /// keel's verification verbs over MCP).
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
    /// One console over every project: which sandboxes are up, what
    /// stage each spec is at per `keel next`, a conversational turn with
    /// any of them, and the two-key stage approval. Local terminal only —
    /// no socket, no port, no daemon.
    Studio {
        /// Show just this project instead of every project on disk.
        #[arg(short = 'p', long = "project")]
        project: Option<String>,
    },
    /// Show (or follow) a running `moor recipe`'s progress — stage
    /// transitions, gate attempts, pauses for approval — from the same
    /// tamper-evident audit chain `moor audit` reads, so you can check
    /// where it's at from a different terminal than the one driving it.
    /// Not deep detail: for full command output, see `moor audit`.
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
        Command::Use { name } => commands::use_cmd::run(&name),
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
