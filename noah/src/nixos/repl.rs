use nh_installable::{FlakeConfig, Installable};
use nix_command::{CommandKind, NixCommand};
use rootcause::{Result, bail};

use super::request::ReplRequest;
use crate::runtime::Env;
use crate::util::get_hostname;
pub(super) fn run(
    request: ReplRequest,
    env: &Env,
    flake_config: &FlakeConfig,
) -> Result<()> {
    let mut target_installable =
        request.installable.resolve_or_default(flake_config)?;

    if matches!(target_installable, Installable::Store { .. }) {
        bail!("Nix doesn't support nix store installables.");
    }

    let hostname = get_hostname(request.hostname)?;

    if let Installable::Flake {
        ref mut attribute, ..
    } = target_installable
        && attribute.is_empty()
    {
        attribute.push(String::from("nixosConfigurations"));
        attribute.push(hostname);
    }

    let status = NixCommand::new(CommandKind::Repl)
        .args(target_installable.to_args())
        .envs(env.child_env())
        .run_with_logs()?;
    if !status.success() {
        bail!("nix repl failed (exit status {status:?})");
    }

    Ok(())
}
