//! bpaf has no public API for completion-script generation; a hidden flag
//! handled by the target applet emits the script. This applet re-execs under
//! that applet's name, and generated scripts query candidates at runtime.

use std::ffi::OsString;
use std::fmt;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::Command;
use std::process::ExitCode;
use std::str::FromStr;

use bpaf::OptionParser;
use bpaf::Parser;
use bpaf::construct;
use bpaf::positional;
use rootcause::prelude::ResultExt as _;
use tracing::debug;

use super::applet::Applet;
use super::applet::invoke;
use super::dispatch::applets;
use super::dispatch::find_applet;

const NAME: &str = "completion";
const SUMMARY: &str = "Generate shell completion scripts for applets";
pub(super) const APPLET: Applet = Applet::new(NAME, SUMMARY, entry);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shell {
    Bash,
    Zsh,
    Fish,
    Elvish,
}

impl Shell {
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

#[derive(Debug, Clone)]
struct CompletionArgs {
    shell: Shell,
    applet: Option<String>,
}

fn args() -> impl Parser<CompletionArgs> {
    let shell = positional::<Shell>("SHELL")
        .help("Target shell: bash, zsh, fish, elvish");
    let applet = positional::<String>("APPLET")
        .help("Applet name (omit to generate for all applets)")
        .optional();
    construct!(CompletionArgs { shell, applet })
}

fn cli() -> OptionParser<CompletionArgs> {
    args().to_options().descr(SUMMARY)
}

fn entry(args: &[OsString]) -> ExitCode {
    invoke(&cli(), NAME, args, |parsed| {
        run(&parsed).map(|()| ExitCode::SUCCESS)
    })
}

fn run(args: &CompletionArgs) -> rootcause::Result<()> {
    let exe = std::env::current_exe()
        .context("Unable to resolve current executable")?;

    let flag = format!("--bpaf-complete-style-{}", args.shell.as_str());

    if let Some(name) = args.applet.as_deref() {
        let applet = find_applet(name).ok_or_else(|| {
            rootcause::report!("unknown applet '{name}'")
        })?;
        return generate_completion(&exe, &flag, args.shell, applet);
    }

    for (index, applet) in applets().enumerate() {
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
        // `argv[0]` routes the child to this applet.
        .arg0(name)
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
    use bpaf::Args;
    use std::assert_matches;

    fn parse(
        items: &[&str],
    ) -> Result<CompletionArgs, bpaf::ParseFailure> {
        cli().run_inner(Args::from(items).set_name(NAME))
    }

    #[test]
    fn parses_shell_and_optional_applet() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::Elvish]
        {
            let name = shell.as_str();
            let parsed = parse(&[name]).unwrap();
            assert_eq!(parsed.shell, shell, "{name}");
            assert!(parsed.applet.is_none(), "{name}");
        }

        let parsed = parse(&["bash", "uuid7"]).unwrap();
        assert_eq!(parsed.shell, Shell::Bash);
        assert_eq!(parsed.applet.as_deref(), Some("uuid7"));
    }

    #[test]
    fn unknown_shell_rejected() {
        assert_matches!(
            parse(&["tcsh"]),
            Err(bpaf::ParseFailure::Stderr(_))
        );
    }
}
