use std::{
    fmt, io,
    path::Path,
    process::{Command as StdCommand, Stdio},
    str,
};

use color_eyre::Result;
use color_eyre::Result as EyreResult;
use color_eyre::eyre;
use color_eyre::eyre::Context;
use color_eyre::eyre::bail;
use regex::Regex;
use semver::Version;
use tracing::debug;
use tracing::info;

#[derive(Debug, Clone, PartialEq)]
pub enum NixVariant {
    Nix,
    Lix,
    DetSys,
}

struct WriteFmt<W: io::Write>(W);

impl<W: io::Write> fmt::Write for WriteFmt<W> {
    fn write_str(&mut self, string: &str) -> fmt::Result {
        self.0.write_all(string.as_bytes()).map_err(|_| fmt::Error)
    }
}

/// Get various information through the nix cli.
/// The returned tuple has 3 elements: the variant, the version,
/// and a list of enabled experimental features.
#[tracing::instrument]
pub fn nix_info() -> EyreResult<(NixVariant, Version, Vec<String>)> {
    use std::process::Command;

    debug!("get nix information from cli");

    let output = Command::new("nix")
        .arg("--version")
        .output()
        .context("Failed to run nix --version")?;

    // The first line of "nix --version" output contains both the
    // variant and the version string.
    let (variant, version) = if let Some(ver_line) =
        String::from_utf8(output.stdout)
            .context("Failed to process nix --version output")?
            .lines()
            .next()
    {
        let ver_line = ver_line.to_lowercase();

        let re: Regex = Regex::new(r"(?<ver>\d+\.\d+\.\d+)").expect(
            "[BUG] Someone doesn't know how to write correct regex",
        );

        let variant = if ver_line.contains("determinate") {
            NixVariant::DetSys
        } else if ver_line.contains("lix") {
            NixVariant::Lix
        } else {
            NixVariant::Nix
        };

        let version = if let Some(cap) = re.captures(&ver_line)
            && let Some(mat) = cap.name("ver")
        {
            Version::parse(mat.as_str())
                .context("Failed to parse version")?
        } else {
            bail!(
                "Failed to get version from nix --versin output. Note that Noah only accounts for stable releases, meaning that versions with -git or -prerelease in it may not match."
            )
        };

        (variant, version)
    } else {
        bail!("nix command didn't produce a meaningful output")
    };

    let output = Command::new("nix")
        .arg("config")
        .arg("show")
        .arg("experimental-features")
        .output()
        .context("Failed to run nix config show")?;

    let features = String::from_utf8(output.stdout)
        .context("Failed to process nix --version output")?
        .split_whitespace()
        .map(|s| s.to_owned())
        .collect();

    debug!(?variant);
    debug!(?version);
    debug!(?features);
    Ok((variant, version, features))
}

/// Prompts the user for ssh key login if needed
pub fn ensure_ssh_key_login() -> Result<()> {
    // ssh-add -L checks if there are any currently usable ssh keys

    if StdCommand::new("ssh-add")
        .arg("-L")
        .stdout(Stdio::null())
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
/// # Returns
///
/// * `Result<String>` - The hostname as a string or an error
pub fn get_hostname() -> Result<String> {
    use color_eyre::eyre::Context;
    Ok(hostname::get()
        .context("Failed to get hostname")?
        .to_str()
        .map_or_else(
            || String::from("unknown-hostname"),
            std::string::ToString::to_string,
        ))
}

/// Self-elevates the current process by re-executing it with sudo
///
/// # Panics
///
/// Panics if the process re-execution with elevated privileges fails.
///
/// # Examples
///
/// ```rust
/// // Elevate the current process to run as root
/// let elevate: fn() -> ! = nh::util::self_elevate;
/// ```
pub fn self_elevate() -> ! {
    use std::os::unix::process::CommandExt;

    let mut cmd = crate::commands::Command::self_elevate_cmd()
        .expect("Failed to create self-elevation command");
    debug!("{:?}", cmd);
    let err = cmd.exec();
    panic!("{}", err);
}

/// Prints the difference between two generations in terms of paths and closure sizes.
///
/// # Arguments
///
/// * `old_generation` - A reference to the path of the old generation.
/// * `new_generation` - A reference to the path of the new generation.
///
/// # Returns
///
/// Returns `Ok(())` if the operation completed successfully, or an error wrapped in `eyre::Result` if something went wrong.
///
/// # Errors
///
/// Returns an error if the closure size thread panics or if writing size differences fails.
pub fn print_dix_diff(
    old_generation: &Path,
    new_generation: &Path,
) -> Result<()> {
    let mut out = WriteFmt(io::stdout());

    // Handle to the thread collecting closure size information.
    let closure_size_handle = dix::spawn_size_diff(
        old_generation.to_path_buf(),
        new_generation.to_path_buf(),
    );

    let wrote =
        dix::write_paths_diffln(&mut out, old_generation, new_generation)
            .unwrap_or_default();

    if let Ok((size_old, size_new)) =
        closure_size_handle.join().map_err(|_| {
            eyre::eyre!("Failed to join closure size computation thread")
        })?
    {
        if size_old == size_new {
            info!("No version or size changes.");
        } else {
            if wrote > 0 {
                println!();
            }
            dix::write_size_diffln(&mut out, size_old, size_new)?;
        }
    }
    Ok(())
}

pub fn list_nixos_names() {}
