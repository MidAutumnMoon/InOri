use std::path::PathBuf;

use super::request::{
    Activation, ActivationAction, ActivationRequest, BuildOptions,
    GenerationsRequest, RebuildCommand, RebuildRequest, ReplRequest,
    RollbackRequest, SpecialisationSelection,
};
use bpaf::{Parser, construct, long, positional};
use nh_installable::InstallableArgs;

use crate::cli::env_bool_strict;
use crate::cli::env_boolish;
use crate::cli::switch_or_env;
use crate::diff::Mode as DiffMode;
use crate::nix_options::NixCliOptions;
use crate::nix_options::nix_build_options_cli;
use crate::nixos::generations::Field;
use crate::remote::Host;
use crate::update::update_cli;

#[derive(Clone, Debug)]
struct ParsedBuildOptions {
    options: BuildOptions,
    commit_lock_file: bool,
    use_substitutes: bool,
}

#[must_use]
fn build_options_cli() -> impl Parser<ParsedBuildOptions> {
    let no_nom = long("no-nom")
        .switch()
        .help("Don't use nix-output-monitor for the build process");
    let out_link = long("out-link")
        .short('o')
        .argument::<PathBuf>("PATH")
        .help(
            "Path to save the result link, defaults to using a temporary \
             directory",
        )
        .optional();
    let diff = long("diff")
        .short('d')
        .argument::<DiffMode>("DIFF")
        .help("Whether to display a package diff")
        .fallback(DiffMode::Auto)
        .display_fallback();
    let nix = nix_build_options_cli();
    let installable = nh_installable::installable_args();

    construct!(no_nom, out_link, diff, nix, installable).map(
        |(no_nom, out_link, diff, nix, installable): (
            bool,
            Option<PathBuf>,
            DiffMode,
            NixCliOptions,
            InstallableArgs,
        )| ParsedBuildOptions {
            options: BuildOptions {
                installable,
                no_nom,
                out_link,
                diff,
                nix: nix.build,
            },
            commit_lock_file: nix.commit_lock_file,
            use_substitutes: nix.use_substitutes,
        },
    )
}

#[must_use]
fn specialisation_cli() -> impl Parser<SpecialisationSelection> {
    let named = long("specialisation")
        .short('s')
        .argument::<String>("NAME")
        .help("Explicitly select some specialisation")
        .map(SpecialisationSelection::Named);
    let base = long("no-specialisation")
        .short('S')
        .help("Ignore specialisations")
        .req_flag(SpecialisationSelection::Base);

    construct!([named, base]).optional().map(|selection| {
        selection.unwrap_or(SpecialisationSelection::Current)
    })
}

#[must_use]
fn rebuild_cli() -> impl Parser<RebuildRequest> {
    let update = update_cli();
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
    let specialisation = specialisation_cli();
    let bypass_root_check = switch_or_env(
        long("bypass-root-check")
            .short('R')
            .help("Don't panic if calling nh as root")
            .switch(),
        env_bool_strict("NH_BYPASS_ROOT_CHECK"),
    );
    let target_host = long("target-host")
        .argument::<Host>("HOST")
        .help(
            "Deploy the built configuration to a different host over SSH",
        )
        .optional();
    let build_host = long("build-host")
        .argument::<Host>("HOST")
        .help("Build the configuration on a different host over SSH")
        .optional();
    let parsed_build = build_options_cli();
    let extra_args = positional::<String>("EXTRA")
        .strict()
        .help("Extra arguments passed to nix build")
        .many();

    construct!(
        update,
        hostname,
        specialisation,
        bypass_root_check,
        target_host,
        build_host,
        parsed_build,
        extra_args
    )
    .map(
        |(
            update,
            hostname,
            specialisation,
            bypass_root_check,
            target_host,
            build_host,
            parsed_build,
            extra_args,
        )| RebuildRequest {
            build: parsed_build.options,
            update,
            hostname,
            specialisation,
            extra_args,
            bypass_root_check,
            target_host,
            build_host,
            commit_lock_file: parsed_build.commit_lock_file,
            use_substitutes: parsed_build.use_substitutes,
        },
    )
}

#[derive(Clone, Copy, Debug)]
struct ActivationFlags {
    dry: bool,
    ask: bool,
    no_validate: bool,
}

#[must_use]
fn activation_flags_cli() -> impl Parser<ActivationFlags> {
    let dry = long("dry")
        .short('n')
        .switch()
        .help("Only print actions, without performing them");
    let ask = switch_or_env(
        long("ask").short('a').help("Ask for confirmation").switch(),
        env_boolish("NH_ASK"),
    );
    let no_validate = switch_or_env(
        long("no-validate")
            .help("Skip pre-activation system validation checks")
            .switch(),
        env_bool_strict("NH_NO_VALIDATE"),
    );

    construct!(ActivationFlags {
        dry,
        ask,
        no_validate,
    })
}

fn activation_request(
    rebuild: RebuildRequest,
    flags: ActivationFlags,
    action: ActivationAction,
) -> RebuildCommand {
    RebuildCommand::Activate(ActivationRequest {
        rebuild,
        activation: Activation {
            action,
            dry: flags.dry,
            ask: flags.ask,
            no_validate: flags.no_validate,
        },
    })
}

#[must_use]
pub fn build_cli() -> impl Parser<RebuildCommand> {
    rebuild_cli().map(RebuildCommand::Build)
}

#[must_use]
pub fn test_cli() -> impl Parser<RebuildCommand> {
    let rebuild = rebuild_cli();
    let flags = activation_flags_cli();
    let show_logs = switch_or_env(
        long("show-activation-logs")
            .help("Show activation logs")
            .switch(),
        env_boolish("NH_SHOW_ACTIVATION_LOGS"),
    );

    construct!(flags, show_logs, rebuild).map(
        |(flags, show_logs, rebuild)| {
            activation_request(
                rebuild,
                flags,
                ActivationAction::Test { show_logs },
            )
        },
    )
}

#[must_use]
pub fn boot_cli() -> impl Parser<RebuildCommand> {
    let rebuild = rebuild_cli();
    let flags = activation_flags_cli();
    let install_bootloader = long("install-bootloader")
        .switch()
        .help("Install the bootloader");

    construct!(flags, install_bootloader, rebuild).map(
        |(flags, install_bootloader, rebuild)| {
            activation_request(
                rebuild,
                flags,
                ActivationAction::Boot { install_bootloader },
            )
        },
    )
}

#[must_use]
pub fn switch_cli() -> impl Parser<RebuildCommand> {
    let rebuild = rebuild_cli();
    let flags = activation_flags_cli();
    let show_logs = switch_or_env(
        long("show-activation-logs")
            .help("Show activation logs")
            .switch(),
        env_boolish("NH_SHOW_ACTIVATION_LOGS"),
    );
    let install_bootloader = long("install-bootloader")
        .switch()
        .help("Install the bootloader");

    construct!(flags, show_logs, install_bootloader, rebuild).map(
        |(flags, show_logs, install_bootloader, rebuild)| {
            activation_request(
                rebuild,
                flags,
                ActivationAction::Switch {
                    show_logs,
                    install_bootloader,
                },
            )
        },
    )
}

#[must_use]
pub fn rollback_cli() -> impl Parser<RollbackRequest> {
    let dry = long("dry")
        .short('n')
        .switch()
        .help("Only print actions, without performing them");
    let ask = switch_or_env(
        long("ask").short('a').help("Ask for confirmation").switch(),
        env_boolish("NH_ASK"),
    );
    let specialisation = specialisation_cli();
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
        .argument::<DiffMode>("DIFF")
        .help("Whether to display a package diff")
        .fallback(DiffMode::Auto)
        .display_fallback();

    construct!(RollbackRequest {
        dry,
        ask,
        specialisation,
        to,
        bypass_root_check,
        diff,
    })
}

#[must_use]
pub fn repl_cli() -> impl Parser<ReplRequest> {
    let hostname = long("hostname")
        .short('H')
        .argument::<String>("HOSTNAME")
        .help(
            "When using a flake installable, select this hostname from \
             nixosConfigurations",
        )
        .optional();
    let installable = nh_installable::installable_args();

    construct!(ReplRequest {
        hostname,
        installable,
    })
}

#[must_use]
pub fn generations_cli() -> impl Parser<GenerationsRequest> {
    let profile = long("profile")
        .short('P')
        .argument::<String>("PROFILE")
        .help("Path to Nix' profiles directory")
        .fallback(String::from("/nix/var/nix/profiles/system"))
        .display_fallback()
        .map(PathBuf::from);
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

    construct!(GenerationsRequest { profile, fields })
}

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
    use std::path::PathBuf;

    use bpaf::{Args, ParseFailure, Parser as _};
    use nh_installable::Installable;
    use nh_installable::InstallableArgs;

    use super::ActivationAction;
    use super::GenerationsRequest;
    use super::RebuildCommand;
    use super::RebuildRequest;
    use super::SpecialisationSelection;
    use super::boot_cli;
    use super::build_cli;
    use super::generations_cli;
    use super::switch_cli;
    use super::test_cli;
    use crate::nixos::generations::Field;

    fn parse_build(
        args: &[&str],
    ) -> std::result::Result<RebuildRequest, String> {
        let options = build_cli().to_options();
        options.check_invariants(false);
        let command = options
            .run_inner(Args::from(args).set_name("test"))
            .map_err(ParseFailure::unwrap_stderr)?;
        match command {
            RebuildCommand::Build(request) => Ok(request),
            RebuildCommand::Activate(_) => Err(String::from(
                "build parser produced activation request",
            )),
        }
    }

    #[test]
    fn extra_args_come_after_double_dash() {
        let request =
            parse_build(&[".", "--", "--option", "a", "b", "-j", "1"])
                .unwrap();

        assert_eq!(request.extra_args, ["--option", "a", "b", "-j", "1"]);
        assert!(request.build.nix.option.is_empty());
        assert!(request.build.nix.max_jobs.is_none());
        match request.build.installable {
            InstallableArgs::Specified(Installable::Flake {
                reference,
                ..
            }) => assert_eq!(reference, "."),
            other => panic!("Expected a flake installable, got {other:?}"),
        }
    }

    #[test]
    fn named_flags_parse_around_positional_installable() {
        let request = parse_build(&["-j", "2", "."]).unwrap();

        assert_eq!(request.build.nix.max_jobs, Some(2));
        assert!(matches!(
            request.build.installable,
            InstallableArgs::Specified(_)
        ));
    }

    #[test]
    fn file_and_installable_positional_combine() {
        let request = parse_build(&["-f", "file.nix", "attr"]).unwrap();

        match request.build.installable {
            InstallableArgs::Specified(Installable::File {
                attribute,
                ..
            }) => assert_eq!(attribute, ["attr"]),
            other => panic!("Expected a file installable, got {other:?}"),
        }
    }

    #[test]
    fn hidden_file_expr_args_parse() {
        let request =
            parse_build(&["-E", "{ pkgs }: pkgs.hello"]).unwrap();

        match request.build.installable {
            InstallableArgs::Specified(Installable::Expression {
                ..
            }) => {}
            other => {
                panic!("Expected an expression installable, got {other:?}")
            }
        }
    }

    #[test]
    fn build_rejects_activation_only_flags() {
        for flag in [
            "--dry",
            "--ask",
            "--no-validate",
            "--show-activation-logs",
            "--install-bootloader",
        ] {
            parse_build(&[flag]).unwrap_err();
        }
    }

    #[test]
    fn action_specific_flags_build_valid_actions() {
        let switch =
            switch_cli()
                .to_options()
                .run_inner(
                    Args::from(
                        &[
                            "--show-activation-logs",
                            "--install-bootloader",
                        ][..],
                    )
                    .set_name("test"),
                )
                .unwrap();
        let RebuildCommand::Activate(switch) = switch else {
            panic!("expected activation request");
        };
        assert!(matches!(
            switch.activation.action,
            ActivationAction::Switch {
                show_logs: true,
                install_bootloader: true
            }
        ));

        let test_error = test_cli().to_options().run_inner(
            Args::from(&["--install-bootloader"][..]).set_name("test"),
        );
        test_error.unwrap_err();

        let boot_error = boot_cli().to_options().run_inner(
            Args::from(&["--show-activation-logs"][..]).set_name("test"),
        );
        boot_error.unwrap_err();
    }

    #[test]
    fn contradictory_specialisation_flags_are_rejected() {
        parse_build(&["--specialisation", "foo", "--no-specialisation"])
            .unwrap_err();
        let request = parse_build(&["--no-specialisation"]).unwrap();
        assert!(matches!(
            request.specialisation,
            SpecialisationSelection::Base
        ));
    }

    #[test]
    fn fields_split_on_commas() {
        let options = generations_cli().to_options();
        options.check_invariants(false);
        let request = options
            .run_inner(
                Args::from(&["--fields", "id,confRev,date"][..])
                    .set_name("test"),
            )
            .unwrap();

        let Some(fields) = request.fields else {
            panic!("fields must be present");
        };
        assert!(matches!(
            fields.as_slice(),
            [Field::Id, Field::Confrev, Field::Date]
        ));
    }

    #[test]
    fn fields_have_canonical_default_profile() {
        let options = generations_cli().to_options();
        let GenerationsRequest { profile, fields } = options
            .run_inner(Args::from(&[] as &[&str]).set_name("test"))
            .unwrap();

        assert!(fields.is_none());
        assert_eq!(profile, PathBuf::from("/nix/var/nix/profiles/system"));
    }
}
