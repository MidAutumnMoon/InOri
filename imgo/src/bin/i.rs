use std::fmt;
use std::process::Command;
use std::str::FromStr;

use bpaf::OptionParser;
use bpaf::Parser as _;
use bpaf::construct;
use bpaf::positional;
use imgo::automation::PlanOpts;
use imgo::automation::RunOpts;
use imgo::automation::create_plan;
use imgo::automation::plan_cli;
use imgo::automation::run_cli;
use imgo::automation::run_plan;
use imgo::pipeline::SharedOpts;
use imgo::pipeline::run_pipeline_pixel;
use imgo::pipeline::run_pipeline_recipe;
use imgo::pipeline::shared_cli;
use imgo::recipe::Recipe;
use imgo::recipe::Step;
use imgo::transcoder::avif::Avif;
use imgo::transcoder::avif::cli as avif_cli;
use imgo::transcoder::jxl::Jxl;
use imgo::transcoder::jxl::cli as jxl_cli;
use imgo::transcoder::magick::CleanScan;
use imgo::transcoder::magick::Denoise;
use imgo::transcoder::magick::clean_scan_cli;
use imgo::transcoder::magick::denoise_cli;
use imgo::transcoder::tomato::Tomato;
use imgo::transcoder::tomato::cli as tomato_cli;
use rlimit::Resource;
use rootcause::Result;
use rootcause::bail;
use rootcause::prelude::ResultExt as _;
use tracing::debug;
use tracing::warn;

/// Content-aware image planning, review, and batch conversion.
#[derive(Debug)]
enum CliOpts {
    /// Phase 1: classify, encode review candidates, then stop for selection.
    Plan(PlanOpts),

    /// Phase 2: apply selected recipes, reusing reviewed representative bytes.
    Run(RunOpts),

    /// Directly encode discovered pictures to AVIF.
    Avif {
        transcoder: Avif,
        shared: SharedOpts,
    },

    /// Directly encode discovered pictures to mathematically lossless JXL.
    Jxl { transcoder: Jxl, shared: SharedOpts },

    /// Apply one `ImageMagick` denoise operation and emit PNG.
    Denoise {
        transcoder: Denoise,
        shared: SharedOpts,
    },

    /// Convert degraded near-bilevel manga scans to one-bit PNG.
    CleanScan {
        transcoder: CleanScan,
        shared: SharedOpts,
    },

    /// 番茄图: scramble/descramble via a Gilbert-curve pixel permutation.
    Tomato { tomato: Tomato, shared: SharedOpts },

    /// Generate shell completion.
    Completion { shell: Shell },
}

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

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "bash" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "fish" => Ok(Self::Fish),
            "elvish" => Ok(Self::Elvish),
            other => Err(format!(
                "expected one of `bash`, `zsh`, `fish`, `elvish`, got `{other}`"
            )),
        }
    }
}

/// bpaf exposes no public completion-script generation API; the script is
/// produced by a hidden `--bpaf-complete-style-<shell>` flag handled inside
/// the parser. Re-exec the current binary with that flag and print the
/// captured script. Generation and runtime completion use separate flags, so
/// the generated script never regenerates itself.
fn run_completion(shell: Shell) -> Result<()> {
    let executable = std::env::current_exe()
        .context("resolve the current executable")?;
    let flag = format!("--bpaf-complete-style-{}", shell.as_str());
    let output = Command::new(&executable)
        .arg(&flag)
        .output()
        .context("re-run the current executable to generate completion")?;
    if !output.status.success() {
        bail!(
            "completion generation failed (exit {:?}):\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let script = String::from_utf8(output.stdout)
        .context("completion output is not valid UTF-8")?;
    print!("{script}");
    Ok(())
}

#[must_use]
fn cli() -> OptionParser<CliOpts> {
    let plan = plan_cli()
        .to_options()
        .descr(
            "Phase 1: classify, encode review candidates, then stop for \
             selection.",
        )
        .command("plan")
        .map(CliOpts::Plan);
    let run = run_cli()
        .to_options()
        .descr(
            "Phase 2: apply selected recipes, reusing reviewed \
             representative bytes.",
        )
        .command("run")
        .map(CliOpts::Run);
    let avif = {
        let transcoder = avif_cli();
        let shared = shared_cli();
        construct!(CliOpts::Avif { transcoder, shared })
            .to_options()
            .descr("Directly encode discovered pictures to AVIF.")
            .command("avif")
            .short('a')
    };
    let jxl = {
        let transcoder = jxl_cli();
        let shared = shared_cli();
        construct!(CliOpts::Jxl { transcoder, shared })
            .to_options()
            .descr(
                "Directly encode discovered pictures to mathematically \
                 lossless JXL.",
            )
            .command("jxl")
            .short('j')
    };
    let denoise = {
        let transcoder = denoise_cli();
        let shared = shared_cli();
        construct!(CliOpts::Denoise { transcoder, shared })
            .to_options()
            .descr(
                "Apply one `ImageMagick` denoise operation and emit PNG.",
            )
            .command("denoise")
            .short('d')
    };
    let clean_scan = {
        let transcoder = clean_scan_cli();
        let shared = shared_cli();
        construct!(CliOpts::CleanScan { transcoder, shared })
            .to_options()
            .descr("Convert degraded near-bilevel manga scans to one-bit PNG.")
            .command("clean-scan")
            .short('c')
    };
    let tomato = {
        let tomato = tomato_cli();
        let shared = shared_cli();
        construct!(CliOpts::Tomato { tomato, shared })
            .to_options()
            .descr(
                "番茄图: scramble/descramble via a Gilbert-curve pixel \
                 permutation.",
            )
            .command("tomato")
            .short('t')
    };
    let completion = {
        let shell = positional::<Shell>("SHELL")
            .help("Target shell: bash, zsh, fish, elvish");
        construct!(CliOpts::Completion { shell })
            .to_options()
            .descr("Generate shell completion.")
            .command("completion")
    };

    construct!([
        plan, run, avif, jxl, denoise, clean_scan, tomato, completion,
    ])
    .to_options()
    .descr("Content-aware image planning, review, and batch conversion.")
    .version(env!("CARGO_PKG_VERSION"))
}

fn main() -> Result<()> {
    let _log_guard = ino_tracing::init_tracing_subscriber();
    let options = cli().run();

    match Resource::NOFILE.get() {
        Ok((_, hard)) => {
            if let Err(error) = Resource::NOFILE.set(hard, hard) {
                warn!(%error, "Failed to raise the open-file limit");
            }
        }
        Err(error) => warn!(%error, "Failed to query the open-file limit"),
    }

    match options {
        CliOpts::Plan(options) => {
            create_plan(&options)?;
            Ok(())
        }
        CliOpts::Run(options) => run_plan(&options),
        CliOpts::Avif { transcoder, shared } => {
            run_pipeline_recipe(
                &shared,
                Recipe::single(Step::Avif(transcoder)),
            )?;
            Ok(())
        }
        CliOpts::Jxl { transcoder, shared } => {
            run_pipeline_recipe(
                &shared,
                Recipe::single(Step::Jxl(transcoder)),
            )?;
            Ok(())
        }
        CliOpts::Denoise { transcoder, shared } => {
            run_pipeline_recipe(
                &shared,
                Recipe::single(Step::Denoise(transcoder)),
            )?;
            Ok(())
        }
        CliOpts::CleanScan { transcoder, shared } => {
            run_pipeline_recipe(
                &shared,
                Recipe::single(Step::CleanScan(transcoder)),
            )?;
            Ok(())
        }
        CliOpts::Tomato { tomato, shared } => {
            run_pipeline_pixel(&shared, &tomato)?;
            Ok(())
        }
        CliOpts::Completion { shell } => {
            debug!(?shell, "Generate shell completion");
            run_completion(shell)
        }
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, clippy::panic, reason = "test assertions")]
mod tests {
    use bpaf::Args;
    use bpaf::ParseFailure;
    use imgo::transcoder::avif::Chroma;

    use super::*;

    fn parse(args: &[&str]) -> std::result::Result<CliOpts, ParseFailure> {
        cli().run_inner(Args::from(args).set_name("i"))
    }

    #[test]
    fn parses_plan_with_default_workspace() {
        let CliOpts::Plan(options) = parse(&["plan"]).unwrap() else {
            panic!("expected the plan command");
        };
        assert_eq!(options.workspace, std::path::PathBuf::from("."));
        assert!(options.output.is_none());
        assert!(!options.force);
    }

    #[test]
    fn parses_run_plan_positional() {
        let CliOpts::Run(options) = parse(&["run", "foo.json"]).unwrap()
        else {
            panic!("expected the run command");
        };
        assert_eq!(options.plan, std::path::PathBuf::from("foo.json"));
    }

    #[test]
    fn parses_transcoder_options() {
        let CliOpts::Avif { transcoder, shared } =
            parse(&["avif", "-q", "70", "--chroma", "420", "page.png"])
                .unwrap()
        else {
            panic!("expected the avif command");
        };
        assert_eq!(transcoder.quality, 70);
        assert_eq!(transcoder.chroma, Chroma::Yuv420);
        assert_eq!(
            shared.manual_selection,
            Some(vec![std::path::PathBuf::from("page.png")])
        );

        let CliOpts::Jxl {
            transcoder: jxl_transcoder,
            ..
        } = parse(&["jxl", "--no-optimize"]).unwrap()
        else {
            panic!("expected the jxl command");
        };
        assert!(!jxl_transcoder.optimize);
    }

    #[test]
    fn manual_selection_is_optional() {
        let CliOpts::Denoise { shared, .. } = parse(&["denoise"]).unwrap()
        else {
            panic!("expected the denoise command");
        };
        assert!(shared.manual_selection.is_none());
    }

    #[test]
    fn parses_command_short_aliases() {
        let CliOpts::CleanScan { transcoder, .. } =
            parse(&["c", "--otsu"]).unwrap()
        else {
            panic!("expected the clean-scan command");
        };
        assert!(transcoder.otsu);
        assert!(transcoder.sharpen);
    }

    #[test]
    fn parses_completion_shell() {
        let CliOpts::Completion { shell } =
            parse(&["completion", "fish"]).unwrap()
        else {
            panic!("expected the completion command");
        };
        assert_eq!(shell, Shell::Fish);
    }

    #[test]
    fn rejects_unknown_flags_and_shells() {
        assert!(matches!(
            parse(&["avif", "--nope"]),
            Err(ParseFailure::Stderr(_))
        ));
        assert!(matches!(
            parse(&["completion", "tcsh"]),
            Err(ParseFailure::Stderr(_))
        ));
        assert!(matches!(parse(&[]), Err(ParseFailure::Stderr(_))));
    }
}
