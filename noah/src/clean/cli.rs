use std::path::PathBuf;
use std::time::Duration;

use bpaf::{Parser, construct, long, positional};

use super::Options;
use super::Request;
use super::Scope;
use crate::cli::env_boolish;
use crate::cli::switch_or_env;

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

#[derive(Debug, Clone)]
struct ProfileRequest {
    options: Options,
    profile: PathBuf,
}

/// Parse the `all`, `user`, and `profile` cleanup scopes into one request.
#[must_use]
pub fn clean_cli() -> impl Parser<Request> {
    let all = clean_options_cli()
        .to_options()
        .descr("Clean all profiles.")
        .command("all")
        .map(|options| Request {
            scope: Scope::All,
            options,
        });
    let user = clean_options_cli()
        .to_options()
        .descr("Clean the current user's profiles.")
        .command("user")
        .map(|options| Request {
            scope: Scope::User,
            options,
        });
    let profile = {
        let options = clean_options_cli();
        let profile = positional::<PathBuf>("PROFILE")
            .help("Which profile to clean");
        construct!(ProfileRequest { options, profile })
            .to_options()
            .descr("Clean a specific profile.")
            .command("profile")
            .map(|request| Request {
                scope: Scope::Profile(request.profile),
                options: request.options,
            })
    };

    construct!([all, user, profile])
}
