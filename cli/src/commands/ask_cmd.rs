use crate::session::{self, Role};
use anyhow::Result;
use std::path::Path;

/// `moor ask` — one chained turn with the project's agent. Thin on
/// purpose: the protocol itself lives in `session.rs`, so the CLI surface
/// and the studio console (spec `studio-multi-project-console`) drive the
/// same code rather than two lookalike copies.
pub fn run(
    name: &str,
    role: &str,
    new_session: bool,
    emit_recipe: Option<&Path>,
    prompt: &[String],
) -> Result<()> {
    let role = Role::parse(role)?;
    let prompt = prompt.join(" ");
    let prompt = prompt.trim();
    if prompt.is_empty() {
        anyhow::bail!(
            "nothing to ask — try `moor ask --role {} -- \"what would it take to ...\"`",
            role.as_str()
        );
    }
    session::ask(name, role, prompt, new_session, emit_recipe)
}
