use bpaf::{Parser, construct, long};
use nh_installable::Installable;
use nix_command::{CommandKind, NixCommand};
use rootcause::{Result, bail};
use tracing::{info, warn};

#[derive(Clone, Debug)]
pub enum Selection {
    All,
    Inputs(Vec<String>),
}

/// Parse the mutually exclusive flake update modes.
#[must_use]
pub fn update_cli() -> impl Parser<Option<Selection>> {
    let all = long("update")
        .short('u')
        .help("Update all flake inputs")
        .req_flag(Selection::All);
    let inputs = long("update-input")
        .short('U')
        .help("Update the specified flake input(s)")
        .argument::<String>("INPUT")
        .some("expected at least one flake input name")
        .map(Selection::Inputs);

    construct!([all, inputs]).optional()
}

/// Update flake inputs for an installable.
///
/// # Errors
///
/// Returns an error if `nix flake update` fails.
pub fn update(
    installable: &Installable,
    selection: &Selection,
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

    let message = match selection {
        Selection::Inputs(inputs) => {
            cmd = cmd.args(inputs);

            let maybe_plural = if inputs.len() > 1 { "s" } else { "" };
            format!(
                "Updating flake input{maybe_plural} {}",
                inputs.join(", ")
            )
        }
        Selection::All => "Updating all flake inputs".to_owned(),
    };

    info!("{message}");

    let status = cmd.arg("--flake").arg(reference).run_with_logs()?;

    if !status.success() {
        bail!("{message} (exit status {status:?})");
    }

    Ok(())
}
