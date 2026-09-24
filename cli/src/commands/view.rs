use crate::{manifest::Manifest, paths, proc};
use anyhow::Result;

/// The three keel-produced artifacts `moor view` knows how to find,
/// mapped to their filename under `.keel/specs/<slug>/`.
fn artifact_filename(artifact: &str) -> Result<&'static str> {
    match artifact {
        "spec" => Ok("spec.md"),
        "plan" => Ok("plan.md"),
        "tasks" => Ok("tasks.md"),
        other => anyhow::bail!("unknown artifact '{other}' — expected spec, plan, or tasks"),
    }
}

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

/// Cheap ANSI highlighting for the plain-markdown files keel writes —
/// headings, list bullets, checkboxes, and code fences — so `moor view`
/// reads better in a terminal without pulling in a markdown-rendering
/// dependency for three known-simple file shapes.
fn highlight(markdown: &str) -> String {
    let mut out = String::new();
    let mut in_code_fence = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
            out.push_str(DIM);
            out.push_str(line);
            out.push_str(RESET);
        } else if in_code_fence {
            out.push_str(DIM);
            out.push_str(line);
            out.push_str(RESET);
        } else if trimmed.starts_with('#') {
            out.push_str(BOLD);
            out.push_str(CYAN);
            out.push_str(line);
            out.push_str(RESET);
        } else if trimmed.starts_with("- [ ]") || trimmed.starts_with("- [x]") {
            out.push_str(YELLOW);
            out.push_str(line);
            out.push_str(RESET);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

pub fn run(project: &str, slug: &str, artifact: &str) -> Result<()> {
    let filename = artifact_filename(artifact)?;
    let m = Manifest::load(&paths::manifest_path(project)?)?;
    let container = m.sandbox_container();
    let path = format!(".keel/specs/{slug}/{filename}");

    let (status, out) = proc::run_capture("docker", &["exec", &container, "cat", &path])?;
    if !status.success() {
        anyhow::bail!(
            "no {filename} found for spec '{slug}' in project '{project}' (looked for {path})"
        );
    }
    print!("{}", highlight(&out));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_artifacts_to_their_filename() {
        assert_eq!(artifact_filename("spec").unwrap(), "spec.md");
        assert_eq!(artifact_filename("plan").unwrap(), "plan.md");
        assert_eq!(artifact_filename("tasks").unwrap(), "tasks.md");
    }

    #[test]
    fn rejects_an_unknown_artifact() {
        let err = artifact_filename("evidence").unwrap_err();
        assert!(err.to_string().contains("evidence"));
    }

    #[test]
    fn highlights_headings_bold_cyan() {
        let out = highlight("# Tasks\nplain line");
        assert_eq!(out, format!("{BOLD}{CYAN}# Tasks{RESET}\nplain line\n"));
    }

    #[test]
    fn highlights_checkbox_lines_yellow() {
        let out = highlight("- [ ] todo\n- [x] done");
        assert_eq!(
            out,
            format!("{YELLOW}- [ ] todo{RESET}\n{YELLOW}- [x] done{RESET}\n")
        );
    }

    #[test]
    fn dims_code_fence_contents_including_the_fences() {
        let out = highlight("```rust\nlet x = 1;\n```");
        assert_eq!(
            out,
            format!("{DIM}```rust{RESET}\n{DIM}let x = 1;{RESET}\n{DIM}```{RESET}\n")
        );
    }

    #[test]
    fn leaves_ordinary_lines_untouched() {
        let out = highlight("just a sentence.");
        assert_eq!(out, "just a sentence.\n");
    }

    #[test]
    fn a_heading_marker_inside_a_code_fence_is_dimmed_not_bolded() {
        let out = highlight("```\n# not a heading\n```");
        assert_eq!(
            out,
            format!("{DIM}```{RESET}\n{DIM}# not a heading{RESET}\n{DIM}```{RESET}\n")
        );
    }
}
