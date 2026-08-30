use crate::nix_command::{CommandKind, NixCommand};
use rootcause::{Result, bail};

use super::request::ReplRequest;
use crate::runtime::Env;
use crate::target::{self, BuildTarget};

pub(super) fn run(request: ReplRequest, env: &Env) -> Result<()> {
    let mut target = target::resolve(request.target, env)?;

    if matches!(target, BuildTarget::StorePath(_)) {
        bail!("Nix doesn't support store path targets.");
    }

    let hostname = match &request.hostname {
        Some(hostname) => hostname.clone(),
        None => env.hostname().to_owned(),
    };

    if let BuildTarget::Flake { attribute, .. } = &mut target
        && attribute.is_empty()
    {
        attribute.push(String::from("nixosConfigurations"));
        attribute.push(hostname);
    }

    let status = NixCommand::new(CommandKind::Repl)
        .args(target.to_args())
        .envs(env.child_env())
        .run_with_logs()?;
    if !status.success() {
        bail!("nix repl failed (exit status {status:?})");
    }

    Ok(())
}
