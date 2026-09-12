use std::path::PathBuf;
use std::time::Duration;

use bpaf::{Parser, construct, long, positional};

use super::CliOpts;
use super::Options;
use super::Scope;

/// Options shared by every cleanup scope.
#[must_use]
fn clean_options_cli() -> impl Parser<Options> {
    let keep = long("keep")
        .short('k')
        .argument::<u32>("KEEP")
        .help("At least keep this number of generations")
        .fallback(1)
        .display_fallback();
    let keep_since = long("keep-since")
        .short('K')
        .argument::<humantime::Duration>("DURATION")
        .help("Keep gcroots and generations newer than this (humantime format)")
        .fallback(Duration::from_secs(0).into())
        .display_fallback();
    let dry = long("dry")
        .short('n')
        .switch()
        .help("Only print actions, without performing them");
    let ask = long("ask").short('a').help("Ask for confirmation").switch();
    let no_gc = long("no-gc")
        .long("nogc")
        .switch()
        .help("Don't run nix store --gc");
    let no_gcroots = long("no-gcroots")
        .long("nogcroots")
        .switch()
        .help("Don't clean gcroots");
    let no_direnv = long("no-direnv")
        .long("nodirenv")
        .switch()
        .help("Don't clean direnv gcroots");
    let optimise = long("optimise")
        .switch()
        .help("Run nix-store --optimise after gc");
    let max = long("max")
        .argument::<String>("MAX")
        .help("Pass --max to nix store gc")
        .optional();
    let keep_one = long("keep-one")
        .switch()
        .help("Keep at least one gcroot per direnv project");
    let cross_filesystems = long("cross-filesystems")
        .short('x')
        .switch()
        .help("Cross filesystem boundaries when scanning gcroots");

    construct!(Options {
        keep,
        keep_since,
        dry,
        ask,
        no_gc,
        no_gcroots,
        no_direnv,
        optimise,
        max,
        keep_one,
        cross_filesystems,
    })
}

/// Parse the `clean` command into one [`CliOpts`]: the standalone gc-root
/// query, or one of the `all`, `user`, and `profile` cleanup scopes.
#[must_use]
pub fn clean_cli() -> impl Parser<CliOpts> {
    let print_gc_roots = long("print-gc-roots")
        .short('P')
        .help("Print GC roots")
        .req_flag(())
        .map(|()| CliOpts::PrintGcRoots);
    let all = clean_options_cli()
        .to_options()
        .descr("Clean all profiles.")
        .command("all")
        .map(|options| CliOpts::Clean {
            scope: Scope::All,
            options,
        });
    let user = clean_options_cli()
        .to_options()
        .descr("Clean the current user's profiles.")
        .command("user")
        .map(|options| CliOpts::Clean {
            scope: Scope::User,
            options,
        });
    let profile = {
        let options = clean_options_cli();
        let profile = positional::<PathBuf>("PROFILE")
            .help("Which profile to clean");
        construct!(options, profile)
            .to_options()
            .descr("Clean a specific profile.")
            .command("profile")
            .map(|(options, profile)| CliOpts::Clean {
                scope: Scope::Profile(profile),
                options,
            })
    };

    construct!([print_gc_roots, all, user, profile])
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Test assertions")]
mod tests {
    use bpaf::Args;
    use bpaf::ParseFailure;
    use bpaf::Parser as _;

    use super::CliOpts;
    use super::Scope;
    use super::clean_cli;

    fn parse(args: &[&str]) -> std::result::Result<CliOpts, String> {
        let options = clean_cli().to_options();
        options.check_invariants(false);
        options
            .run_inner(Args::from(args).set_name("nh"))
            .map_err(ParseFailure::unwrap_stderr)
    }

    #[test]
    fn print_gc_roots_is_a_bare_clean_level_query() {
        assert!(matches!(parse(&["-P"]).unwrap(), CliOpts::PrintGcRoots));
        assert!(matches!(
            parse(&["--print-gc-roots"]).unwrap(),
            CliOpts::PrintGcRoots
        ));
    }

    #[test]
    fn scoped_cleanups_still_parse() {
        assert!(matches!(
            parse(&["all"]).unwrap(),
            CliOpts::Clean {
                scope: Scope::All,
                ..
            }
        ));
        assert!(matches!(
            parse(&["user", "--dry"]).unwrap(),
            CliOpts::Clean {
                scope: Scope::User,
                ..
            }
        ));
        assert!(matches!(
            parse(&["profile", "/nix/var/nix/profiles/system"]).unwrap(),
            CliOpts::Clean {
                scope: Scope::Profile(_),
                ..
            }
        ));
    }

    #[test]
    fn print_gc_roots_does_not_combine_with_a_scope() {
        // The query is a clean-level alternative to the scope commands, so
        // either order leaves an item unconsumed.
        assert!(parse(&["all", "-P"]).unwrap_err().contains("`-P`"));
        assert!(parse(&["-P", "all"]).unwrap_err().contains("`all`"));
    }

    #[test]
    fn bare_clean_without_query_or_scope_fails() {
        let err = parse(&[]).unwrap_err();
        assert!(err.contains("--print-gc-roots"));
        assert!(err.contains("COMMAND"));
    }
}
