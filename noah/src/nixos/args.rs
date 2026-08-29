use std::path::PathBuf;

use bpaf::{construct, long, positional, Parser};
use nh_installable::InstallableArgs;

use crate::args::DiffType;
use crate::args::NixBuildPassthrough;
use crate::args::env_bool_strict;
use crate::args::env_boolish;
use crate::args::passthrough_cli;
use crate::args::switch_or_env;
use crate::nixos::generations::Field;
use crate::remote::Host;
use crate::update::Update;
use crate::update::update_cli;

#[derive(Clone, Debug)]
pub struct Rebuild {
    pub common: CommonRebuild,

    pub update_args: Update,

    /// When using a flake installable, select this hostname from
    /// nixosConfigurations.
    ///
    /// When unspecified, defaults to the local hostname for local
    /// deployments, and hostname of the target machine for remote
    /// deployments (see --target-host).
    pub hostname: Option<String>,

    /// Explicitly select some specialisation.
    pub specialisation: Option<String>,

    /// Ignore specialisations.
    pub no_specialisation: bool,

    /// Install bootloader for switch and boot commands.
    pub install_bootloader: bool,

    /// Extra arguments passed to nix build.
    pub extra_args: Vec<String>,

    /// Don't panic if calling nh as root.
    pub bypass_root_check: bool,

    /// Deploy the built configuration to a different host over SSH.
    pub target_host: Option<Host>,

    /// Build the configuration on a different host over SSH.
    pub build_host: Option<Host>,

    /// Skip pre-activation system validation checks.
    pub no_validate: bool,
}

#[derive(Clone, Debug)]
pub struct RebuildActivate {
    pub rebuild: Rebuild,

    /// Show activation logs.
    pub show_activation_logs: bool,
}

#[derive(Clone, Debug)]
pub struct Rollback {
    /// Only print actions, without performing them.
    pub dry: bool,

    /// Ask for confirmation.
    pub ask: bool,

    /// Explicitly select some specialisation.
    pub specialisation: Option<String>,

    /// Ignore specialisations.
    pub no_specialisation: bool,

    /// Rollback to a specific generation number (defaults to previous
    /// generation).
    pub to: Option<u64>,

    /// Don't panic if calling nh as root.
    pub bypass_root_check: bool,

    /// Whether to display a package diff.
    pub diff: DiffType,
}

#[derive(Clone, Debug)]
pub struct CommonRebuild {
    /// Only print actions, without performing them.
    pub dry: bool,

    /// Ask for confirmation.
    pub ask: bool,

    pub installable: InstallableArgs,

    /// Don't use nix-output-monitor for the build process.
    pub no_nom: bool,

    /// Path to save the result link, defaults to using a temporary directory.
    pub out_link: Option<PathBuf>,

    /// Whether to display a package diff.
    pub diff: DiffType,

    pub passthrough: NixBuildPassthrough,
}

#[derive(Clone, Debug)]
pub struct Repl {
    pub installable: InstallableArgs,

    /// When using a flake installable, select this hostname from
    /// nixosConfigurations.
    pub hostname: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Generations {
    /// Path to Nix' profiles directory.
    pub profile: Option<String>,

    /// Comma-delimited list of field(s) to display.
    pub fields: Option<Vec<Field>>,
}

/// CLI parser for [`CommonRebuild`].
#[must_use]
pub fn common_rebuild_cli() -> impl Parser<CommonRebuild> {
    let dry = long("dry")
        .short('n')
        .switch()
        .help("Only print actions, without performing them");
    let ask = switch_or_env(
        long("ask").short('a').help("Ask for confirmation").switch(),
        env_boolish("NH_ASK"),
    );
    let no_nom = long("no-nom")
        .switch()
        .help("Don't use nix-output-monitor for the build process");
    let out_link = long("out-link")
        .short('o')
        .argument::<PathBuf>("PATH")
        .help("Path to save the result link, defaults to using a temporary directory")
        .optional();
    let diff = long("diff")
        .short('d')
        .argument::<DiffType>("DIFF")
        .help("Whether to display a package diff")
        .fallback(DiffType::Auto)
        .display_fallback();
    let passthrough = passthrough_cli();
    let installable = nh_installable::installable_args();

    construct!(CommonRebuild {
        dry,
        ask,
        no_nom,
        out_link,
        diff,
        passthrough,
        installable,
    })
}

/// CLI parser for [`Rebuild`].
#[must_use]
pub fn rebuild_cli() -> impl Parser<Rebuild> {
    let update_args = update_cli();
    let hostname = long("hostname")
        .short('H')
        .argument::<String>("HOSTNAME")
        .help(
            "When using a flake installable, select this hostname from \
             nixosConfigurations.\n\nWhen unspecified, defaults to the local \
             hostname for local deployments, and hostname of the target \
             machine for remote deployments (see --target-host)",
        )
        .optional();
    let specialisation = long("specialisation")
        .short('s')
        .argument::<String>("NAME")
        .help("Explicitly select some specialisation")
        .optional();
    let no_specialisation = long("no-specialisation")
        .short('S')
        .switch()
        .help("Ignore specialisations");
    let install_bootloader = long("install-bootloader")
        .switch()
        .help("Install bootloader for switch and boot commands");
    let bypass_root_check = switch_or_env(
        long("bypass-root-check")
            .short('R')
            .help("Don't panic if calling nh as root")
            .switch(),
        env_bool_strict("NH_BYPASS_ROOT_CHECK"),
    );
    let target_host = long("target-host")
        .argument::<Host>("HOST")
        .help("Deploy the built configuration to a different host over SSH")
        .optional();
    let build_host = long("build-host")
        .argument::<Host>("HOST")
        .help("Build the configuration on a different host over SSH")
        .optional();
    let no_validate = switch_or_env(
        long("no-validate")
            .help("Skip pre-activation system validation checks")
            .switch(),
        env_bool_strict("NH_NO_VALIDATE"),
    );
    let common = common_rebuild_cli();
    let extra_args = positional::<String>("EXTRA")
        .strict()
        .help("Extra arguments passed to nix build")
        .many();

    construct!(Rebuild {
        update_args,
        hostname,
        specialisation,
        no_specialisation,
        install_bootloader,
        bypass_root_check,
        target_host,
        build_host,
        no_validate,
        common,
        extra_args,
    })
}

/// CLI parser for [`RebuildActivate`].
#[must_use]
pub fn rebuild_activate_cli() -> impl Parser<RebuildActivate> {
    let rebuild = rebuild_cli();
    let show_activation_logs = switch_or_env(
        long("show-activation-logs")
            .help("Show activation logs")
            .switch(),
        env_boolish("NH_SHOW_ACTIVATION_LOGS"),
    );

    construct!(RebuildActivate {
        show_activation_logs,
        rebuild,
    })
}

/// CLI parser for [`Rollback`].
#[must_use]
pub fn rollback_cli() -> impl Parser<Rollback> {
    let dry = long("dry")
        .short('n')
        .switch()
        .help("Only print actions, without performing them");
    let ask = switch_or_env(
        long("ask").short('a').help("Ask for confirmation").switch(),
        env_boolish("NH_ASK"),
    );
    let specialisation = long("specialisation")
        .short('s')
        .argument::<String>("NAME")
        .help("Explicitly select some specialisation")
        .optional();
    let no_specialisation = long("no-specialisation")
        .short('S')
        .switch()
        .help("Ignore specialisations");
    let to = long("to")
        .short('t')
        .argument::<u64>("GENERATION")
        .help(
            "Rollback to a specific generation number (defaults to previous \
             generation)",
        )
        .optional();
    let bypass_root_check = switch_or_env(
        long("bypass-root-check")
            .short('R')
            .help("Don't panic if calling nh as root")
            .switch(),
        env_bool_strict("NH_BYPASS_ROOT_CHECK"),
    );
    let diff = long("diff")
        .short('d')
        .argument::<DiffType>("DIFF")
        .help("Whether to display a package diff")
        .fallback(DiffType::Auto)
        .display_fallback();

    construct!(Rollback {
        dry,
        ask,
        specialisation,
        no_specialisation,
        to,
        bypass_root_check,
        diff,
    })
}

/// CLI parser for [`Repl`].
#[must_use]
pub fn repl_cli() -> impl Parser<Repl> {
    let hostname = long("hostname")
        .short('H')
        .argument::<String>("HOSTNAME")
        .help(
            "When using a flake installable, select this hostname from \
             nixosConfigurations",
        )
        .optional();
    let installable = nh_installable::installable_args();

    construct!(Repl {
        hostname,
        installable,
    })
}

/// CLI parser for [`Generations`].
#[must_use]
pub fn generations_cli() -> impl Parser<Generations> {
    let profile = long("profile")
        .short('P')
        .argument::<String>("PROFILE")
        .help("Path to Nix' profiles directory")
        .fallback(String::from("/nix/var/nix/profiles/system"))
        .display_fallback()
        .map(Some);
    let fields = long("fields")
        .argument::<String>("FIELDS")
        .help("Comma-delimited list of field(s) to display")
        .many()
        .parse(|values: Vec<String>| {
            if values.is_empty() {
                return Ok(None);
            }
            parse_field_selection(&values).map(Some)
        });

    construct!(Generations { profile, fields })
}

/// Split comma-delimited `--fields` values into [`Field`]s.
fn parse_field_selection(
    values: &[String],
) -> std::result::Result<Vec<Field>, String> {
    let mut fields = Vec::new();
    for value in values {
        for part in value.split(',') {
            fields.push(part.parse::<Field>()?);
        }
    }
    Ok(fields)
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::panic,
    clippy::wildcard_enum_match_arm,
    reason = "Test assertions"
)]
mod tests {
    use bpaf::{Args, ParseFailure, Parser as _};
    use nh_installable::Installable;
    use nh_installable::InstallableArgs;

    use super::Rebuild;
    use super::generations_cli;
    use super::rebuild_cli;
    use crate::nixos::generations::Field;

    fn parse_rebuild(
        args: &[&str],
    ) -> std::result::Result<Rebuild, String> {
        let options = rebuild_cli().to_options();
        options.check_invariants(false);
        options
            .run_inner(Args::from(args).set_name("test"))
            .map_err(ParseFailure::unwrap_stderr)
    }

    #[test]
    fn extra_args_come_after_double_dash() {
        let args =
            parse_rebuild(&[".", "--", "--option", "a", "b", "-j", "1"])
                .unwrap();

        assert_eq!(args.extra_args, ["--option", "a", "b", "-j", "1"]);
        assert!(args.common.passthrough.option.is_empty());
        assert!(args.common.passthrough.max_jobs.is_none());
        match args.common.installable {
            InstallableArgs::Specified(Installable::Flake {
                reference,
                ..
            }) => assert_eq!(reference, "."),
            other => panic!("Expected a flake installable, got {other:?}"),
        }
    }

    #[test]
    fn named_flags_parse_around_positional_installable() {
        let args = parse_rebuild(&["-j", "2", "."]).unwrap();

        assert_eq!(args.common.passthrough.max_jobs, Some(2));
        assert!(matches!(
            args.common.installable,
            InstallableArgs::Specified(_)
        ));
    }

    #[test]
    fn file_and_installable_positional_combine() {
        let args = parse_rebuild(&["-f", "file.nix", "attr"]).unwrap();

        match args.common.installable {
            InstallableArgs::Specified(Installable::File {
                attribute,
                ..
            }) => assert_eq!(attribute, ["attr"]),
            other => panic!("Expected a file installable, got {other:?}"),
        }
    }

    #[test]
    fn hidden_file_expr_args_parse() {
        let args = parse_rebuild(&["-E", "{ pkgs }: pkgs.hello"]).unwrap();

        match args.common.installable {
            InstallableArgs::Specified(Installable::Expression {
                ..
            }) => {}
            other => {
                panic!("Expected an expression installable, got {other:?}")
            }
        }
    }

    #[test]
    fn fields_split_on_commas() {
        let options = generations_cli().to_options();
        options.check_invariants(false);
        let args = options
            .run_inner(
                Args::from(&["--fields", "id,confRev,date"][..])
                    .set_name("test"),
            )
            .unwrap();

        let Some(fields) = args.fields else {
            panic!("fields must be present");
        };
        assert!(matches!(
            fields.as_slice(),
            [Field::Id, Field::Confrev, Field::Date]
        ));
    }

    #[test]
    fn fields_default_to_none() {
        let options = generations_cli().to_options();
        let args = options
            .run_inner(Args::from(&[] as &[&str]).set_name("test"))
            .unwrap();

        assert!(args.fields.is_none());
        assert_eq!(
            args.profile.as_deref(),
            Some("/nix/var/nix/profiles/system")
        );
    }
}
