//! `moor spec new` and `moor spec push`: write a spec on the host, with
//! whatever help you like, then send it into the sandbox.
//!
//! Only the spec's text crosses, and only host → sandbox. The one thing
//! read back is the id the sandbox assigned, which must match a strict
//! shape before it is used: a sandbox that has been tampered with gets
//! no way to put its own text into a file on the host.

use super::next_cmd::Target;
use crate::{audit, manifest, paths, proc};
use anyhow::{Context, Result};
use std::path::Path;

/// Marks the rules block `moor spec new` puts at the top of a starter file.
/// Stripped by `moor spec push`, so it never counts against the spec.
const RULES_START: &str = "<!-- moor:rules";

/// What a spec must satisfy to pass its checks, for whoever writes it.
/// Mirrors the pipeline's own authoring prompt, with its default limits.
const RULES: &str = "<!-- moor:rules
Write this spec from what you want the change to do. This file is all an
AI assistant needs; it doesn't need to read the project's code. Then run
`moor spec push <this file>`. This block is removed when you push.

To pass its checks, the spec needs:
1. The front matter below, with `scope` listing the paths the change may
   touch (globs).
2. At most 12 acceptance criteria, each a `### AC-n <short title>` heading.
3. Each criterion states ONE requirement in exactly one of these forms:
     THE SYSTEM SHALL <response>
     WHEN <trigger> THE SYSTEM SHALL <response>
     WHILE <state> THE SYSTEM SHALL <response>
     IF <condition> THEN THE SYSTEM SHALL <response>
     WHERE <feature> THE SYSTEM SHALL <response>
   `THE SYSTEM SHALL` is upper case. Never write \"should\".
4. Each criterion is followed by at least one `oracle:` line:
     oracle: cmd `<shell command>` exit <code>
     oracle: test <test identifier>
     oracle: human <what a reviewer must judge>
   Prefer one that runs; use `human` only when nothing can.
5. No vague words in criteria (appropriate, efficient, robust, handle,
   support, reasonable, fast, should, etc). Name observable behaviour and
   concrete numbers instead.
6. The whole file stays under 250 lines.
-->
";

/// A starter spec for `slug`, rules first.
pub fn starter(slug: &str, title: &str, scope: &[String]) -> String {
    let scope = if scope.is_empty() {
        vec!["src/**".to_string()]
    } else {
        scope.to_vec()
    };
    let scope_yaml: String = scope.iter().map(|s| format!("  - \"{s}\"\n")).collect();
    format!(
        "{RULES}---\n\
         slug: {slug}\n\
         status: draft\n\
         scope:\n{scope_yaml}\
         budget:\n\
         \x20 criteria: 8\n\
         \x20 lines: 120\n\
         ---\n\
         \n\
         # {title}\n\
         \n\
         ## Context\n\
         \n\
         _Why this change, and what is true today that should not be._\n\
         \n\
         ## Acceptance criteria\n\
         \n\
         ### AC-1 Replace this with one observable behaviour\n\
         \n\
         WHEN <trigger> THE SYSTEM SHALL <observable response>.\n\
         \n\
         oracle: cmd `<command that proves it>` exit 0\n\
         \n\
         ## Out of scope\n\
         \n\
         _What this change deliberately does not do._\n"
    )
}

/// `content` without the leading rules block, if it has one.
fn strip_rules(content: &str) -> &str {
    let trimmed = content.trim_start();
    if !trimmed.starts_with(RULES_START) {
        return content;
    }
    match trimmed.find("-->") {
        Some(end) => trimmed[end + 3..].trim_start(),
        None => content,
    }
}

/// A spec file as written on the host, ready to send.
#[derive(Debug, PartialEq)]
struct Draft {
    slug: String,
    title: String,
    /// The file without its rules block.
    text: String,
}

fn humanise(slug: &str) -> String {
    let mut out = slug.replace('-', " ");
    if let Some(c) = out.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    out
}

/// Splits `---` front matter from the body. `None` if there isn't any.
pub(crate) fn front_matter(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    Some((&rest[..end], &rest[end + 5..]))
}

fn parse(content: &str) -> Result<Draft> {
    let text = strip_rules(content);
    let (front, body) =
        front_matter(text).context("the spec needs front matter between `---` lines at the top")?;
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(front).context("the spec's front matter isn't valid YAML")?;
    let slug = yaml
        .get("slug")
        .and_then(|v| v.as_str())
        .context("the spec's front matter needs a `slug:` (its short name)")?
        .to_string();
    manifest::validate_name(&slug).context("the spec's `slug`")?;
    let title = body
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .map(|t| t.trim().to_string())
        .unwrap_or_else(|| humanise(&slug));
    Ok(Draft {
        slug,
        title,
        text: text.to_string(),
    })
}

/// The sandbox-assigned `id` and `schema` from a spec's front matter —
/// accepted only in their exact expected shapes (`SPEC-` and digits;
/// `keel.spec/` and digits), since this is the one piece of sandbox
/// output this command lets through.
pub(crate) fn read_identity(sandbox_spec: &str) -> Option<(String, String)> {
    let (front, _) = front_matter(sandbox_spec)?;
    let field = |key: &str| {
        front
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .map(|v| v.trim().to_string())
    };
    let digits = |s: &str| !s.is_empty() && s.len() <= 6 && s.bytes().all(|b| b.is_ascii_digit());
    let id = field("id:").filter(|v| v.strip_prefix("SPEC-").is_some_and(digits))?;
    let schema = field("schema:").filter(|v| v.strip_prefix("keel.spec/").is_some_and(digits))?;
    Some((id, schema))
}

/// `text` with its `id` and `schema` set to the sandbox's, whatever the
/// host copy said.
fn with_identity(text: &str, id: &str, schema: &str) -> String {
    let Some((front, body)) = front_matter(text) else {
        return text.to_string();
    };
    let kept: String = front
        .lines()
        .filter(|l| !l.starts_with("id:") && !l.starts_with("schema:"))
        .map(|l| format!("{l}\n"))
        .collect();
    format!("---\nid: {id}\nschema: {schema}\n{kept}---\n{body}")
}

/// `moor spec new <name>`: write a starter spec on the host. Touches no
/// sandbox, so it works before a project is even up.
pub fn new(slug: &str, title: Option<String>, scope: &[String]) -> Result<()> {
    manifest::validate_name(slug).context("the spec's name")?;
    let file = format!("{slug}.spec.md");
    if Path::new(&file).exists() {
        anyhow::bail!(
            "{file} already exists — edit it, or send it in:\n\n  Next:  moor spec push {file}"
        );
    }
    let title = title.unwrap_or_else(|| humanise(slug));
    std::fs::write(&file, starter(slug, &title, scope))
        .with_context(|| format!("writing {file}"))?;
    println!("Wrote {file}.\n");
    println!("  Write the spec in it, by hand or with your AI assistant. The rules");
    println!("  are at the top of the file; it doesn't need the project's code.\n");
    println!("  Next:  moor spec push {file}");
    Ok(())
}

/// Runs `argv` in the sandbox, capturing its output, logged like any exec.
fn capture(t: &Target, argv: &[&str]) -> Result<(bool, String)> {
    let container = t.m.sandbox_container();
    let mut args = vec!["exec", container.as_str()];
    args.extend_from_slice(argv);
    let (status, out) = proc::run_capture_combined("docker", &args)?;
    let logged: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
    audit::log_exec(&t.name, &t.m, "spec", &logged, status.code())?;
    Ok((status.success(), out))
}

/// `moor spec push <file>`: send a spec written on the host into the
/// sandbox, make it the active spec, and run its checks.
pub fn push(explicit: Option<String>, file: &Path) -> Result<()> {
    let content =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let draft = parse(&content)?;
    let t = Target::resolve(explicit)?;
    let flag = t
        .flag
        .as_ref()
        .map(|p| format!(" --project {p}"))
        .unwrap_or_default();
    let dest = format!("/workspace/.keel/specs/{}/spec.md", draft.slug);

    // A new spec is registered first, so the sandbox assigns its id.
    let (exists, _) = capture(&t, &["test", "-f", &dest])?;
    if !exists {
        let (_, out) = capture(
            &t,
            &["keel", "spec", "new", &draft.slug, "--title", &draft.title],
        )?;
        if !capture(&t, &["test", "-f", &dest])?.0 {
            anyhow::bail!(
                "couldn't create spec '{}' in the sandbox:\n{out}",
                draft.slug
            );
        }
    }
    let (_, current) = capture(&t, &["cat", &dest])?;
    let (id, schema) = read_identity(&current)
        .with_context(|| format!("couldn't read spec '{}''s id from the sandbox", draft.slug))?;

    let staged = paths::project_dir(&t.name)?.join("spec-push.md");
    std::fs::write(&staged, with_identity(&draft.text, &id, &schema))?;
    let container = t.m.sandbox_container();
    let write = format!("cat > {dest}");
    let argv = ["exec", "-i", container.as_str(), "sh", "-c", write.as_str()];
    let status = proc::run_with_stdin_file("docker", &argv, &staged);
    std::fs::remove_file(&staged).ok();
    let status = status?;
    let logged: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
    audit::log_exec(&t.name, &t.m, "spec", &logged, status.code())?;
    proc::require_success("writing the spec into the sandbox", status)?;

    std::fs::write(paths::active_spec_path(&t.name)?, &draft.slug)?;
    println!(
        "==> sent {} as spec '{}' ({id}); checking it\n",
        file.display(),
        draft.slug
    );

    let checked = super::keel_cmd::run(&t.name, &["gate".into(), "g0".into(), draft.slug.clone()]);
    if checked.is_err() {
        anyhow::bail!(
            "the spec didn't pass its checks yet. Fix what's listed above in {}, then send it again.\n\n  Next:  moor spec push{flag} {}",
            file.display(),
            file.display()
        );
    }
    println!();
    t.print_guidance(&t.report()?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SANDBOX_SPEC: &str =
        "---\nid: SPEC-0007\nslug: login\nschema: keel.spec/1\nstatus: draft\n---\n\n# Login\n";

    #[test]
    fn a_starter_parses_back_to_its_slug_and_title() {
        let text = starter("login-endpoint", "Add login", &["api/**".into()]);
        let d = parse(&text).unwrap();
        assert_eq!(d.slug, "login-endpoint");
        assert_eq!(d.title, "Add login");
        assert!(
            !d.text.contains("moor:rules"),
            "rules block must be stripped"
        );
        assert!(d.text.starts_with("---\nslug: login-endpoint\n"));
        assert!(d.text.contains("  - \"api/**\""));
    }

    #[test]
    fn a_starter_defaults_its_scope() {
        assert!(starter("a", "A", &[]).contains("  - \"src/**\""));
    }

    #[test]
    fn a_file_without_front_matter_is_refused() {
        let err = parse("# Just a title\n").unwrap_err().to_string();
        assert!(err.contains("front matter"), "{err}");
    }

    #[test]
    fn a_bad_slug_is_refused() {
        assert!(parse("---\nslug: ../etc\n---\n# x\n").is_err());
        assert!(parse("---\nstatus: draft\n---\n# x\n").is_err());
    }

    #[test]
    fn the_title_falls_back_to_the_slug() {
        let d = parse("---\nslug: rate-limits\n---\nno heading\n").unwrap();
        assert_eq!(d.title, "Rate limits");
    }

    #[test]
    fn identity_is_read_only_in_its_exact_shape() {
        assert_eq!(
            read_identity(SANDBOX_SPEC),
            Some(("SPEC-0007".into(), "keel.spec/1".into()))
        );
        for bad in [
            "---\nid: SPEC-0007; rm -rf /\nschema: keel.spec/1\n---\n",
            "---\nid: SPEC-\nschema: keel.spec/1\n---\n",
            "---\nid: SPEC-0007\nschema: evil/1\n---\n",
            "---\nid: SPEC-0007\n---\n",
            "no front matter",
        ] {
            assert_eq!(read_identity(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_sandbox_identity_replaces_the_hosts() {
        let host = "---\nid: SPEC-9999\nslug: login\nstatus: draft\n---\n\n# Login\n";
        let out = with_identity(host, "SPEC-0007", "keel.spec/1");
        assert_eq!(
            out,
            "---\nid: SPEC-0007\nschema: keel.spec/1\nslug: login\nstatus: draft\n---\n\n# Login\n"
        );
    }

    #[test]
    fn a_file_without_the_rules_block_is_untouched() {
        let text = "---\nslug: a\n---\n# A\n";
        assert_eq!(strip_rules(text), text);
    }
}
