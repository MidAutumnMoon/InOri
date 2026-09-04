//! The `repl` command: load a NixOS configuration into `nix repl`.

use bpaf::Parser;
use bpaf::construct;
use bpaf::long;
use rootcause::Result;
use rootcause::bail;

use crate::nix::command::Kind;
use crate::nix::command::NixCommand;
use crate::runtime::Env;
use crate::target::{self, BuildTarget};

#[derive(Clone, Debug)]
pub struct Request {
    pub target: Option<BuildTarget>,
    pub hostname: Option<String>,
}

/// Parse the `repl` command.
#[must_use]
pub fn cli() -> impl Parser<Request> {
    let hostname = long("hostname")
        .short('H')
        .argument::<String>("HOSTNAME")
        .help(
            "When using a flake, select this hostname from \
             nixosConfigurations",
        )
        .optional();
    let target = target::parser();

    construct!(Request { hostname, target })
}

/// Run a repl request.
///
/// # Errors
///
/// Returns an error if target resolution or the repl invocation fails.
pub fn run(request: Request, env: &Env) -> Result<()> {
    let mut target = target::resolve(request.target, env)?;

    if matches!(target, BuildTarget::StorePath(_)) {
        bail!("Nix doesn't support store path targets.");
    }

    let hostname = request
        .hostname
        .as_ref()
        .map_or_else(|| env.hostname().to_owned(), String::clone);

    if let BuildTarget::Flake { attribute, .. } = &mut target
        && attribute.is_empty()
    {
        attribute.push(String::from("nixosConfigurations"));
        attribute.push(hostname);
    }

    let status = NixCommand::new(Kind::Repl)
        .args(target.to_args())
        .envs(env.child_env())
        .run_with_logs()?;
    if !status.success() {
        bail!("nix repl failed (exit status {status:?})");
    }

    Ok(())
}
