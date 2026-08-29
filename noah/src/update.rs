use bpaf::{construct, long, Parser};
use nh_installable::Installable;
use nix_command::{CommandKind, NixCommand};
use rootcause::{Result, bail};
use tracing::{info, warn};

#[derive(Clone, Debug)]
pub struct Update {
    /// Update all flake inputs.
    pub update_all: bool,

    /// Update the specified flake input(s).
    pub update_input: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
enum UpdateChoice {
    All,
    Inputs(Vec<String>),
}

/// CLI parser for [`Update`]: `--update` and `--update-input` are mutually
/// exclusive, and both are optional.
#[expect(clippy::module_name_repetitions, reason = "clearer, mirrors clean_cli/search_cli")]
#[must_use]
pub fn update_cli() -> impl Parser<Update> {
    let update_all = long("update")
        .short('u')
        .help("Update all flake inputs")
        .req_flag(UpdateChoice::All);
    let update_input = long("update-input")
        .short('U')
        .help("Update the specified flake input(s)")
        .argument::<String>("INPUT")
        .some("expected at least one flake input name")
        .map(UpdateChoice::Inputs);
    let update = construct!([update_all, update_input]).optional();

    update.map(|choice| match choice {
        Some(UpdateChoice::All) => Update {
            update_all: true,
            update_input: None,
        },
        Some(UpdateChoice::Inputs(inputs)) => Update {
            update_all: false,
            update_input: Some(inputs),
        },
        None => Update {
            update_all: false,
            update_input: None,
        },
    })
}

/// Update flake inputs for an installable.
///
/// # Errors
///
/// Returns an error if `nix flake update` fails.
pub fn update(
    installable: &Installable,
    inputs: Option<Vec<String>>,
    commit_lock_file: bool,
) -> Result<()> {
    let Installable::Flake { reference, .. } = installable else {
        warn!(
            "Only flake installables can be updated, {} is not supported",
            installable.str_kind()
        );
        return Ok(());
    };

    let mut cmd = NixCommand::new(CommandKind::Flake).arg("update");

    if commit_lock_file {
        cmd = cmd.arg("--commit-lock-file");
    }

    let message = match inputs {
        Some(inputs) if !inputs.is_empty() => {
            cmd = cmd.args(&inputs);

            let maybe_plural = if inputs.len() > 1 { "s" } else { "" };
            format!(
                "Updating flake input{maybe_plural} {}",
                inputs.join(", ")
            )
        }
        _ => "Updating all flake inputs".to_owned(),
    };

    info!("{message}");

    let status = cmd.arg("--flake").arg(reference).run_with_logs()?;

    if !status.success() {
        bail!("{message} (exit status {status:?})");
    }

    Ok(())
}
