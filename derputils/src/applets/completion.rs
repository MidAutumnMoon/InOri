//! `completion` — generate shell completion scripts for applets.
//!
//! bpaf exposes no public API for completion-script generation; the scripts are
//! produced by hidden flags handled inside the parser. This applet re-execs the
//! current executable with those flags and prints the captured output.
//!
//! Script generation and runtime completion use separate flags, so generated
//! scripts query candidates at runtime rather than regenerating themselves.

use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::process::Command;
use std::process::ExitCode;
use std::str::FromStr;

use bpaf::Args;
use bpaf::OptionParser;
use bpaf::Parser as _;
use bpaf::construct;
use bpaf::positional;
use rootcause::prelude::ResultExt as _;
use tracing::debug;

use super::Applet;
use super::RunFailure;
use super::all_applets;
use super::find_applet;

const NAME: &str = "completion";
const SUMMARY: &str = "Generate shell completion scripts for applets";
pub(super) const APPLET: Applet = Applet::new(NAME, SUMMARY, applet_main);

/// Target shell for completion script generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shell {
    Bash,
    Zsh,
    Fish,
    Elvish,
}

impl Shell {
    /// Canonical shell name, shared by `Display` and `FromStr`.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::Elvish => "elvish",
        }
    }
}

impl fmt::Display for Shell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Shell {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "bash" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "fish" => Ok(Self::Fish),
            "elvish" => Ok(Self::Elvish),
            _ => Err(format!(
                "unknown shell '{s}'; expected one of: bash, zsh, fish, elvish"
            )),
        }
    }
}

/// Parsed arguments for the `completion` applet.
#[derive(Debug, Clone)]
struct CompletionArgs {
    shell: Shell,
    applet: Option<String>,
}

#[must_use]
fn cli() -> OptionParser<CompletionArgs> {
    let shell = positional::<Shell>("SHELL")
        .help("Target shell: bash, zsh, fish, elvish");
    let applet = positional::<String>("APPLET")
        .help("Applet name (omit to generate for all applets)")
        .optional();
    construct!(CompletionArgs { shell, applet })
        .to_options()
        .descr(SUMMARY)
}

fn applet_main(args: &[OsString]) -> Result<ExitCode, RunFailure> {
    let args = cli()
        .run_inner(Args::from(args).set_name(NAME))
        .map_err(RunFailure::Cli)?;
    run(&args).map_err(RunFailure::Applet)?;
    Ok(ExitCode::SUCCESS)
}

fn run(args: &CompletionArgs) -> rootcause::Result<()> {
    let exe = std::env::current_exe()
        .context("Unable to resolve current executable")?;

    // Depends only on the shell, not the applet — build once.
    let flag = format!("--bpaf-complete-style-{}", args.shell.as_str());

    if let Some(name) = args.applet.as_deref() {
        // Resolve before printing anything.
        let applet = find_applet(name).ok_or_else(|| {
            rootcause::report!("unknown applet '{name}'")
        })?;
        return generate_completion(&exe, &flag, args.shell, applet);
    }

    for (index, applet) in all_applets().enumerate() {
        if index > 0 {
            println!();
        }
        println!("# completion for: {}", applet.name());
        generate_completion(&exe, &flag, args.shell, applet)?;
    }
    Ok(())
}

fn generate_completion(
    exe: &Path,
    flag: &str,
    shell: Shell,
    applet: &Applet,
) -> rootcause::Result<()> {
    let name = applet.name();
    debug!(applet = name, shell = %shell, "generating completion");
    let output = Command::new(exe)
        .arg(name)
        .arg(flag)
        .output()
        .context("Unable to execute self for completion generation")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        rootcause::bail!(
            "completion generation failed for '{name}' with status {}\n{}",
            output.status,
            stderr.trim()
        );
    }
    let script = String::from_utf8(output.stdout)
        .context("Completion output was not valid UTF-8")?;
    // bpaf scripts are already newline-terminated; `println!` would double them.
    print!("{script}");
    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Tests")]
mod test {
    use super::*;
    use std::assert_matches;

    fn parse(args: &[&str]) -> Result<CompletionArgs, bpaf::ParseFailure> {
        cli().run_inner(Args::from(args).set_name(NAME))
    }

    #[test]
    fn parses_shell_and_applet() {
        let args = parse(&["bash", "uuid7"]).unwrap();
        assert_eq!(args.shell, Shell::Bash);
        assert_eq!(args.applet.as_deref(), Some("uuid7"));
    }

    #[test]
    fn applet_is_optional() {
        let args = parse(&["fish"]).unwrap();
        assert_eq!(args.shell, Shell::Fish);
        assert!(args.applet.is_none());
    }

    #[test]
    fn unknown_shell_rejected() {
        assert_matches!(
            parse(&["tcsh"]),
            Err(bpaf::ParseFailure::Stderr(_))
        );
    }

    #[test]
    fn stray_argument_after_two_positionals_rejected() {
        assert_matches!(
            parse(&["bash", "uuid7", "extra"]),
            Err(bpaf::ParseFailure::Stderr(_))
        );
    }

    #[test]
    fn shell_from_str_round_trip() {
        assert_eq!(Shell::from_str("bash").unwrap(), Shell::Bash);
        assert_eq!(Shell::from_str("zsh").unwrap(), Shell::Zsh);
        assert_eq!(Shell::from_str("fish").unwrap(), Shell::Fish);
        assert_eq!(Shell::from_str("elvish").unwrap(), Shell::Elvish);
        Shell::from_str("nope").unwrap_err();
    }
}
