//! Flake input updates performed before a rebuild (`--update`,
//! `--update-input`).

use bpaf::Parser;
use bpaf::construct;
use bpaf::long;
use rootcause::Result;
use rootcause::bail;
use tracing::info;
use tracing::warn;

use crate::nix::command::Kind;
use crate::nix::command::NixCommand;
use crate::target::BuildTarget;

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

/// Update flake inputs for a target.
///
/// # Errors
///
/// Returns an error if `nix flake update` fails.
pub fn run(
    target: &BuildTarget,
    selection: &Selection,
    commit_lock_file: bool,
) -> Result<()> {
    let BuildTarget::Flake { reference, .. } = target else {
        warn!(
            "Only flake targets can be updated, {} is not supported",
            target.kind()
        );
        return Ok(());
    };

    let mut cmd = NixCommand::new(Kind::Flake).arg("update");

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
