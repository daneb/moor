use super::run_cmd;
use anyhow::Result;

/// Builds the argv `moor pipeline` hands to `run_cmd`: `keel` (the pipeline tool inside the sandbox) prepended to
/// whatever the operator typed. Pulled out from `run` so the "keel goes
/// first" rule is unit-testable without a container to exec into.
fn build_argv(args: &[String]) -> Result<Vec<String>> {
    if args.is_empty() {
        anyhow::bail!("usage: moor pipeline [--project <name>] <keel-args...>");
    }
    let mut cmd = vec!["keel".to_string()];
    cmd.extend(args.iter().cloned());
    Ok(cmd)
}

/// Sugar for `moor run <project> -- keel <args...>`: since keel ships
/// natively in every sandbox image, spelling it out after `--` on every
/// call is pure boilerplate. `moor pipeline <args...>` (with `--project` to
/// disambiguate when more than one project exists) drops both the `--`
/// and the repeated `keel`.
pub fn run(project: &str, args: &[String]) -> Result<()> {
    let cmd = build_argv(args)?;
    run_cmd::run(project, &cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepends_keel_to_the_given_args() {
        let argv = build_argv(&["gate".into(), "g1".into(), "blast-radius".into()]).unwrap();
        assert_eq!(argv, vec!["keel", "gate", "g1", "blast-radius"]);
    }

    #[test]
    fn empty_args_is_an_error() {
        let err = build_argv(&[]).unwrap_err();
        assert!(err.to_string().contains("usage"));
    }

    #[test]
    fn preserves_hyphenated_flags_in_order() {
        let argv = build_argv(&[
            "approve".into(),
            "greet-name".into(),
            "--stage".into(),
            "spec".into(),
        ])
        .unwrap();
        assert_eq!(
            argv,
            vec!["keel", "approve", "greet-name", "--stage", "spec"]
        );
    }
}
