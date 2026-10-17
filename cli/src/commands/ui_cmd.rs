use anyhow::Result;

/// `moor ui` — the browser console. Thin: everything is `crate::ui`.
pub fn run(port: u16, no_open: bool) -> Result<()> {
    crate::ui::run(port, !no_open)
}
