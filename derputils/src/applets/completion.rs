//! `completion` — generate shell completion scripts for applets.
//!
//! bpaf's completion generators are private and only reachable by invoking the
//! binary with hidden `--bpaf-complete-style-<shell>` flags. This applet re-execs
//! the current executable once per target applet and prints the captured script.

use std::ffi::OsString;
use std::fmt;
use std::process::Command;
use std::str::FromStr;

use bpaf::Args;
use bpaf::OptionParser;
use bpaf::Parser;
use bpaf::construct;
use bpaf::positional;
use rootcause::prelude::ResultExt;
use tracing::debug;

use crate::applet::RunFailure;
use crate::APPLETS;

pub const NAME: &str = "completion";

/// Target shell for completion script generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Elvish,
}

impl Shell {
    /// The `--bpaf-complete-style-<name>` suffix bpaf expects.
    const fn flag(self) -> &'static str {
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
        f.write_str(self.flag())
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
pub struct CompletionArgs {
    pub shell: Shell,
    pub applet: Option<String>,
}

#[must_use]
pub fn cli() -> OptionParser<CompletionArgs> {
    let shell = positional::<Shell>("SHELL")
        .help("Target shell: bash, zsh, fish, elvish");
    let applet = positional::<String>("APPLET")
        .help("Applet name (omit to generate for all applets)")
        .optional();
    construct!(CompletionArgs { shell, applet })
        .to_options()
        .descr("Generate shell completion scripts for applets")
        .version(env!("CARGO_PKG_VERSION"))
}

pub fn applet_main(args: &[OsString]) -> Result<(), RunFailure> {
    let args = cli()
        .run_inner(Args::from(args).set_name(NAME))
        .map_err(RunFailure::Cli)?;
    run(&args).map_err(RunFailure::Applet)
}

fn run(args: &CompletionArgs) -> rootcause::Result<()> {
    let exe = std::env::current_exe()
        .context("Unable to resolve current executable")?;

    // Resolve the target applet names up front so we bail before printing
    // anything on an unknown name.
    let targets: Vec<&'static str> = match args.applet.as_deref() {
        Some(name) => {
            let applet = APPLETS
                .iter()
                .find(|a| a.name == name)
                .ok_or_else(|| rootcause::report!("unknown applet '{name}'"))?;
            vec![applet.name]
        }
        None => APPLETS.iter().map(|a| a.name).collect(),
    };

    for (i, &name) in targets.iter().enumerate() {
        if targets.len() > 1 {
            if i > 0 {
                println!();
            }
            println!("# completion for: {name}");
        }
        debug!(applet = name, shell = %args.shell, "generating completion");
        let flag = format!("--bpaf-complete-style-{}", args.shell.flag());
        let output = Command::new(&exe)
            .arg(name)
            .arg(&flag)
            .output()
            .context("Unable to execute self for completion generation")?;
        if !output.status.success() {
            rootcause::bail!(
                "completion generation failed for '{name}' with status {}",
                output.status
            );
        }
        let script = String::from_utf8(output.stdout)
            .context("Completion output was not valid UTF-8")?;
        print!("{script}");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

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
        assert!(matches!(
            parse(&["tcsh"]),
            Err(bpaf::ParseFailure::Stderr(_))
        ));
    }

    #[test]
    fn stray_argument_after_two_positionals_rejected() {
        assert!(matches!(
            parse(&["bash", "uuid7", "extra"]),
            Err(bpaf::ParseFailure::Stderr(_))
        ));
    }

    #[test]
    fn shell_from_str_round_trip() {
        assert_eq!(Shell::from_str("bash").unwrap(), Shell::Bash);
        assert_eq!(Shell::from_str("zsh").unwrap(), Shell::Zsh);
        assert_eq!(Shell::from_str("fish").unwrap(), Shell::Fish);
        assert_eq!(Shell::from_str("elvish").unwrap(), Shell::Elvish);
        assert!(Shell::from_str("nope").is_err());
    }
}
