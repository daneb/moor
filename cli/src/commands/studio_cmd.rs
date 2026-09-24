use anyhow::Result;

/// `moor studio` — the console over every project at once. Thin: the
/// console itself is `crate::studio`, and the project list comes from the
/// same on-disk enumeration `moor status` uses.
pub fn run(only: Option<String>) -> Result<()> {
    let names = match only {
        Some(name) => {
            crate::manifest::validate_name(&name)?;
            vec![name]
        }
        None => crate::paths::all_project_names()?,
    };
    crate::studio::run(names)
}
