use crate::paths;
use anyhow::Result;

pub fn run(name: &str) -> Result<()> {
    if !paths::manifest_path(name)?.exists() {
        anyhow::bail!("no project '{name}' — run `moor new {name}` first");
    }
    super::compose_up(name)?;
    println!("'{name}' is up.\n");
    println!("{}", super::next_cmd::next_hint(name));
    Ok(())
}
