use std::os::unix::process::CommandExt as _;
use std::process::Command as StdCommand;
use std::process::Stdio;

use nix::unistd::gethostname;
use rootcause::Result;
use rootcause::prelude::ResultExt as _;
use rootcause::report;
use tracing::debug;

use crate::command::Command;
use crate::command::Elevation;
use crate::remote::SshConfig;
use crate::runtime::Env;

/// Prompts the user for ssh key login if needed.
///
/// # Errors
///
/// Returns an error if:
/// - The `ssh-add -L` command fails to execute
/// - The `ssh-add` command fails to spawn or complete
///
/// # Note
///
/// This func is a no-op when no SSH agent socket is configured (which is
/// unlikely but possible), i.e., when no `SSH_AUTH_SOCK` is set. This behaviour
/// is valid, and SSH can authenticate via the keys in `~/.ssh` or via
/// `~/.ssh/config` without an agent. NH should be able to handle the
/// case without erroring.
pub fn ensure_ssh_key_login(ssh_config: &SshConfig) -> Result<()> {
    if !ssh_config.has_usable_agent() {
        debug!("SSH agent socket not available, skipping ssh-add check");
        return Ok(());
    }

    // ssh-add -L checks if there are any currently usable ssh keys
    if StdCommand::new("ssh-add")
        .arg("-L")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success()
    {
        return Ok(());
    }

    StdCommand::new("ssh-add")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?
        .wait()?;

    Ok(())
}

/// Gets the hostname of the current system.
///
/// # Arguments
///
/// * `supplied_hostname` - An optional hostname provided by the user.
///
/// # Returns
///
/// * `Ok(String)` with the resolved hostname.
/// * `Err` if no hostname is supplied and fetching the system hostname fails.
///
/// # Errors
///
/// Returns an error if:
/// - No hostname is supplied and the system hostname cannot be retrieved
#[deprecated(note = "Use hostname from Env instead.")]
pub fn get_hostname(supplied_hostname: Option<String>) -> Result<String> {
    if let Some(hostname) = supplied_hostname {
        return Ok(hostname);
    }

    gethostname()
        .context("Failed to get hostname, and no hostname supplied")?
        .into_string()
        .map_err(|host| {
            report!("Hostname contains invalid UTF-8: {host:?}")
        })
}

/// Self-elevates the current process by re-executing it with `sudo`.
///
/// # Panics
///
/// Panics if the process re-execution with elevated privileges fails.
#[expect(
    clippy::panic,
    clippy::expect_used,
    reason = "re-exec failure is fatal; the `# Panics` section documents the contract"
)]
pub fn self_elevate(elevation: &Elevation, env: &Env) -> ! {
    let mut cmd = Command::self_elevate_cmd(elevation, env)
        .expect("Failed to create self-elevation command");
    debug!("{:?}", cmd);

    let err = cmd.exec();
    panic!("{err}");
}
