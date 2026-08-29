use std::path::PathBuf;
use std::time::Duration;

use bpaf::{construct, long, positional, Parser};

use crate::args::env_boolish;
use crate::args::switch_or_env;

// Needed a struct to have multiple sub-subcommands
#[derive(Debug, Clone)]
pub struct CleanProxy {
    pub command: CleanMode,
}

#[derive(Debug, Clone)]
pub enum CleanMode {
    /// Clean all profiles.
    All(Clean),
    /// Clean the current user's profiles.
    User(Clean),
    /// Clean a specific profile.
    Profile(CleanProfile),
}

#[derive(Clone, Debug)]
pub struct Clean {
    /// At least keep this number of generations.
    pub keep: u32,

    /// At least keep gcroots and generations in this time range since now.
    ///
    /// See the documentation of humantime for possible formats: <https://docs.rs/humantime/latest/humantime/fn.parse_duration.html>.
    pub keep_since: humantime::Duration,

    /// Only print actions, without performing them.
    pub dry: bool,

    /// Ask for confirmation.
    pub ask: bool,

    /// Don't run nix store --gc.
    pub no_gc: bool,

    /// Don't clean gcroots.
    pub no_gcroots: bool,

    /// Don't clean direnv gcroots.
    pub no_direnv: bool,

    /// Run nix-store --optimise after gc.
    pub optimise: bool,

    /// Pass --max to nix store gc.
    pub max: Option<String>,

    /// Keep at least one gcroot per direnv project.
    pub keep_one: bool,

    /// Cross filesystem boundaries when scanning gcroots.
    pub cross_filesystems: bool,
}

#[derive(Debug, Clone)]
pub struct CleanProfile {
    pub common: Clean,

    /// Which profile to clean.
    pub profile: PathBuf,
}

/// CLI parser for the [`Clean`] options shared by all clean modes.
#[must_use]
fn clean_args_cli() -> impl Parser<Clean> {
    let keep = long("keep")
        .short('k')
        .argument::<u32>("KEEP")
        .help("At least keep this number of generations")
        .fallback(1)
        .display_fallback();
    let keep_since = long("keep-since")
        .short('K')
        .argument::<humantime::Duration>("DURATION")
        .help(
            "At least keep gcroots and generations in this time range since \
             now.\n\nSee the documentation of humantime for possible formats: \
             https://docs.rs/humantime/latest/humantime/fn.parse_duration.html",
        )
        .fallback(Duration::from_secs(0).into())
        .display_fallback();
    let dry = long("dry")
        .short('n')
        .switch()
        .help("Only print actions, without performing them");
    let ask = switch_or_env(
        long("ask").short('a').help("Ask for confirmation").switch(),
        env_boolish("NH_ASK"),
    );
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

    construct!(Clean {
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

/// CLI parser for [`CleanProfile`].
#[must_use]
fn clean_profile_cli() -> impl Parser<CleanProfile> {
    let common = clean_args_cli();
    let profile = positional::<PathBuf>("PROFILE").help("Which profile to clean");

    construct!(CleanProfile { common, profile })
}

/// CLI parser for [`CleanProxy`]: the `all`, `user`, and `profile`
/// sub-subcommands.
#[must_use]
pub fn clean_cli() -> impl Parser<CleanProxy> {
    let all = clean_args_cli()
        .to_options()
        .descr("Clean all profiles.")
        .command("all")
        .map(CleanMode::All);
    let user = clean_args_cli()
        .to_options()
        .descr("Clean the current user's profiles.")
        .command("user")
        .map(CleanMode::User);
    let profile = clean_profile_cli()
        .to_options()
        .descr("Clean a specific profile.")
        .command("profile")
        .map(CleanMode::Profile);
    let command = construct!([all, user, profile]);

    command.map(|command| CleanProxy { command })
}
