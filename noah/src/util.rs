use std::{
    ffi::OsString,
    os::unix::process::CommandExt,
    process::{Command as StdCommand, Stdio},
};

use color_eyre::{
    Result,
    eyre::{self, Context, bail, eyre},
};
use nix_command::{CommandKind, NixCommand};
use tracing::debug;

use crate::command::{
    Command, ElevationStrategy, SubprocessEnv, SudoConfig,
};

fn format_argv(argv: &[OsString]) -> String {
    argv.iter()
        .map(|arg| {
            let arg = arg.to_string_lossy().into_owned();
            shlex::try_quote(&arg)
                .map_or_else(|_| arg.clone(), std::borrow::Cow::into_owned)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn capture_nix_stdout(command: NixCommand) -> Result<String> {
    let argv = command.argv();
    let command_text = format_argv(&argv);
    let output = command
        .output()
        .wrap_err_with(|| format!("Failed to run {command_text}"))?;

    if !output.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            bail!(
                "{command_text} failed (exit status {:?})",
                output.exit_status
            );
        }
        bail!(
            "{command_text} failed (exit status {:?})\nstderr:\n{stderr}",
            output.exit_status
        );
    }

    String::from_utf8(output.stdout).wrap_err_with(|| {
        format!("{command_text} produced non-UTF-8 stdout")
    })
}

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
pub fn ensure_ssh_key_login() -> Result<()> {
    // No usable agent socket means ssh-add has nothing to talk to.
    let agent_available = std::env::var_os("SSH_AUTH_SOCK")
        .is_some_and(|s| std::path::Path::new(&s).exists());
    if !agent_available {
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

/// Gets the hostname of the current system
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
pub fn get_hostname(supplied_hostname: Option<String>) -> Result<String> {
    use color_eyre::eyre::Context;

    if let Some(h) = supplied_hostname {
        return Ok(h);
    }

    nix::unistd::gethostname()
        .context("Failed to get hostname, and no hostname supplied")?
        .into_string()
        .map_err(|_| eyre::eyre!("Hostname contains invalid UTF-8"))
}

/// Self-elevates the current process by re-executing it with `sudo`
///
/// # Panics
///
/// Panics if the process re-execution with elevated privileges fails.
#[allow(clippy::panic, clippy::expect_used)]
pub fn self_elevate(
    strategy: ElevationStrategy,
    subprocess_env: &SubprocessEnv,
    sudo_config: &SudoConfig,
) -> ! {
    let mut cmd =
        Command::self_elevate_cmd(strategy, subprocess_env, sudo_config)
            .expect("Failed to create self-elevation command");
    debug!("{:?}", cmd);

    let err = cmd.exec();
    panic!("{err}");
}

/// Gets the available image variants for a non-flake installable.
///
/// This function uses nix-instantiate to evaluate the available image
/// variants from a Nix expression or file, matching the behavior of
/// nixos-rebuild's `get_build_image_variants` function.
///
/// # Arguments
///
/// * `installable` - The original installable to evaluate
/// * `hostname` - The hostname to use for the configuration
///
/// # Returns
///
/// * `Result<Vec<String>>` - A vector of available image variant names
///
/// # Errors
///
/// Returns an error if:
/// - The nix-instantiate command fails
/// - The JSON output cannot be parsed
/// - The installable does not have images attribute
pub fn get_build_image_variants(
    installable: &nh_installable::Installable,
    hostname: &str,
) -> Result<Vec<String>> {
    let expr = match installable {
        nh_installable::Installable::File { path, .. } => {
            format!(
                r#"
let
  value = import "{}";
  set = if builtins.isFunction value then value {{}} else value;
  config = set.nixosConfigurations."{hostname}" or set;
in
  builtins.attrNames config.config.system.build.images
        "#,
                path.display(),
            )
        }
        nh_installable::Installable::Expression { expression, .. } => {
            format!(
                r#"
let
  value = {expression};
  set = if builtins.isFunction value then value {{}} else value;
  config = set.nixosConfigurations."{hostname}" or set;
in
  builtins.attrNames config.config.system.build.images
        "#
            )
        }
        _ => {
            return Err(eyre!(
                "get_build_image_variants only supports file and expression \
         installables"
            ));
        }
    };

    let result = capture_nix_stdout(
        NixCommand::nix_instantiate()
            .arg("--eval")
            .arg("--strict")
            .arg("--json")
            .arg("--expr")
            .arg(expr),
    )?;

    let variants: Vec<String> = serde_json::from_str(&result)
        .wrap_err("Failed to parse image variants JSON")?;

    Ok(variants)
}

/// Gets the available image variants for a flake installable.
///
/// This function uses nix eval to evaluate the available image
/// variants from a flake.
///
/// # Arguments
///
/// * `installable` - The flake installable to evaluate
///
/// # Returns
///
/// * `Result<Vec<String>>` - A vector of available image variant names
///
/// # Errors
///
/// Returns an error if:
/// - The nix eval command fails
/// - The JSON output cannot be parsed
/// - The flake installable does not have images attribute
pub fn get_build_image_variants_flake(
    installable: &nh_installable::Installable,
) -> Result<Vec<String>> {
    let result = capture_nix_stdout(
        NixCommand::new(CommandKind::Eval)
            .arg("--json")
            .args(installable.to_args())
            .arg("--apply")
            .arg("builtins.attrNames"),
    )?;

    let variants: Vec<String> = serde_json::from_str(&result)
        .wrap_err("Failed to parse image variants JSON")?;

    Ok(variants)
}

//#[cfg(test)]
//#[expect(
//    clippy::expect_used,
//    clippy::unwrap_used,
//    reason = "Fine in tests"
//)]
//mod tests {
//    use nh_installable::Installable;
//
//    use super::*;
//
//    #[test]
//    fn test_get_build_image_variants_expression() {
//        let installable = Installable::Expression {
//            expression: r"
//{
//  nixosConfigurations.test = {
//    config.system.build.images = {
//      iso = {};
//      disk = {};
//      container = {};
//    };
//  };
//}
//      "
//            .to_string(),
//            attribute: vec![],
//        };
//
//        let result = get_build_image_variants(&installable, "test");
//        assert!(result.is_ok());
//
//        let variants = result.unwrap();
//        assert_eq!(variants.len(), 3);
//        assert!(variants.contains(&"iso".to_string()));
//        assert!(variants.contains(&"disk".to_string()));
//        assert!(variants.contains(&"container".to_string()));
//    }
//
//    #[test]
//    fn test_get_build_image_variants_file() {
//        let test_file = tempfile::Builder::new()
//            .prefix("nh-test")
//            .tempfile()
//            .expect("Failed to create temp file");
//        let test_content = r#"
//{
//  nixosConfigurations.test = {
//    config.system.build.images = {
//      iso = "test-iso";
//      disk = "test-disk";
//      container = "test-container";
//    };
//  };
//}
//"#;
//
//        std::fs::write(&test_file, test_content)
//            .expect("Failed to write test file");
//
//        let installable = Installable::File {
//            path: test_file.path().to_path_buf(),
//            attribute: vec![],
//        };
//
//        let result = get_build_image_variants(&installable, "test");
//        assert!(result.is_ok());
//
//        let variants = result.unwrap();
//        assert_eq!(variants.len(), 3);
//        assert!(variants.contains(&"iso".to_string()));
//        assert!(variants.contains(&"disk".to_string()));
//        assert!(variants.contains(&"container".to_string()));
//    }
//
//    #[test]
//    fn test_get_build_image_variants_flake() {
//        use std::fs;
//
//        let test_dir = tempfile::Builder::new()
//            .prefix("nh-test")
//            .tempdir()
//            .expect("Failed to create temp dir");
//
//        // Canonicalize to resolve symlinks
//        let canonical = test_dir
//            .path()
//            .canonicalize()
//            .expect("Failed to canonicalize temp dir");
//        let test_file = canonical.join("flake.nix");
//        let test_content = r"
//{
//  outputs = _: {
//    nixosConfigurations.test.config.system.build.images = {
//      iso = { };
//      disk = { };
//      container = { };
//    };
//  };
//}
//";
//        fs::write(&test_file, test_content)
//            .expect("Failed to write test file");
//
//        let installable = Installable::Flake {
//            reference: format!("path:{}", canonical.display()),
//            attribute: vec![
//                "nixosConfigurations".to_owned(),
//                "test".to_string(),
//                "config".to_string(),
//                "system".to_string(),
//                "build".to_string(),
//                "images".to_string(),
//            ],
//        };
//
//        let result = get_build_image_variants_flake(&installable);
//
//        assert!(result.is_ok());
//
//        let variants = result.unwrap();
//        assert_eq!(variants.len(), 3);
//        assert!(variants.contains(&"iso".to_string()));
//        assert!(variants.contains(&"disk".to_string()));
//        assert!(variants.contains(&"container".to_string()));
//    }
//}
