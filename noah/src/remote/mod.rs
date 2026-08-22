use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::io::Read as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, LazyLock, atomic::Ordering};
use std::time::Duration;

use crate::command::ElevationStrategy;
use crate::command::SudoConfig;
use crate::runtime::RuntimeEnv;
use nh_installable::Installable;
use nix_command::CommandKind;
use nix_command::NixCommand;
use rootcause::Report;
use rootcause::Result;
use rootcause::bail;
use rootcause::prelude::ResultExt as _;
use rootcause::report;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use subprocess::Exec;
use subprocess::Job;
use subprocess::Redirection;
use tracing::debug;
use tracing::info;
use tracing::warn;

pub mod copy;
pub mod dix;

use copy::copy_closure_between_hosts;
use copy::copy_closure_from;
use copy::copy_to_remote;

/// SSH and remote-execution settings derived from the startup environment.
#[derive(Debug, Clone)]
pub struct SshConfig {
    /// `NH_SSHOPTS` (preferred) or `NIX_SSHOPTS` (legacy), shell-split.
    user_opts: Vec<String>,
    /// `NH_REMOTE_CLEANUP` — defaults to `true` when unset; `false` when "0".
    cleanup_remote: bool,
    control_dir: PathBuf,
    agent_socket: Option<PathBuf>,
    nixos_no_check: Option<String>,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            user_opts: Vec::new(),
            cleanup_remote: true,
            control_dir: PathBuf::from("/tmp")
                .join(format!("nh-ssh-{}", std::process::id())),
            agent_socket: None,
            nixos_no_check: None,
        }
    }
}

impl SshConfig {
    /// Parse remote-execution settings from a startup environment snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if the selected SSH options contain unmatched shell
    /// quoting.
    pub fn from_env(env: &RuntimeEnv) -> Result<Self> {
        let control_base = env
            .var_os("XDG_RUNTIME_DIR")
            .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);

        Ok(Self {
            user_opts: env.shell_words("NH_SSHOPTS", "NIX_SSHOPTS")?,
            cleanup_remote: env
                .var("NH_REMOTE_CLEANUP")
                .is_none_or(|value| value != "0"),
            control_dir: control_base
                .join(format!("nh-ssh-{}", std::process::id())),
            agent_socket: env.var_os("SSH_AUTH_SOCK").map(PathBuf::from),
            nixos_no_check: env
                .non_empty_var("NIXOS_NO_CHECK")
                .map(str::to_owned),
        })
    }

    pub(crate) fn has_usable_agent(&self) -> bool {
        self.agent_socket.as_deref().is_some_and(Path::exists)
    }
}

static PASSWORD_CACHE: LazyLock<Mutex<HashMap<String, SecretString>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn get_cached_password(host: &str) -> Result<Option<SecretString>> {
    let guard = PASSWORD_CACHE
        .lock()
        .map_err(|_poisoned| report!("Password cache lock poisoned"))?;
    Ok(guard.get(host).cloned())
}

fn cache_password(host: &str, password: SecretString) -> Result<()> {
    PASSWORD_CACHE
        .lock()
        .map_err(|_poisoned| report!("Password cache lock poisoned"))?
        .insert(host.to_owned(), password);
    Ok(())
}

/// Global flag indicating whether a SIGINT (Ctrl+C) was received.
static INTERRUPTED: LazyLock<Arc<AtomicBool>> =
    LazyLock::new(|| Arc::new(AtomicBool::new(false)));

/// Return the shared interrupt flag.
fn get_interrupt_flag() -> &'static Arc<AtomicBool> {
    &INTERRUPTED
}

/// Cache for signal handler registration status.
static HANDLER_REGISTERED: OnceLock<()> = OnceLock::new();

/// Builds a remote command string with proper elevation handling.
///
/// Constructs the command to execute on the remote host, wrapping it with
/// the appropriate elevation program (sudo/doas/etc) based on the strategy.
///
/// # Arguments
/// * `strategy` - Optional elevation strategy to use
/// * `base_cmd` - The base command to execute
///
/// # Returns
/// The complete command string to execute on the remote.
///
/// # Errors
/// Returns error if:
/// - Elevation program cannot be resolved
/// - Elevation program name cannot be determined
fn build_remote_command(
    strategy: Option<&ElevationStrategy>,
    base_cmd: &str,
    runtime_env: &RuntimeEnv,
    sudo_config: &SudoConfig,
) -> Result<String> {
    if let Some(strategy) = strategy {
        if matches!(strategy, ElevationStrategy::None) {
            return Ok(base_cmd.to_owned());
        }

        let program = strategy.resolve(runtime_env)?;
        let program_name = program
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                report!("Failed to determine elevation program name")
            })?;

        // Use just the program name on the remote host
        // so that the remote system resolves it via its own PATH
        match (program_name, strategy) {
            // sudo passwordless: use --non-interactive to fail if password required
            ("sudo", ElevationStrategy::Passwordless) => {
                Ok(remote_sudo_command(
                    "--non-interactive",
                    base_cmd,
                    sudo_config,
                ))
            }
            ("sudo", _) => Ok(remote_sudo_command(
                "--prompt= --stdin",
                base_cmd,
                sudo_config,
            )),
            // doas passwordless: use -n flag (non-interactive)
            ("doas", ElevationStrategy::Passwordless) => {
                Ok(format!("doas -n {base_cmd}"))
            }
            ("doas", _) => {
                bail!(
                    "doas does not support stdin password input for remote deployment. \
           Use --elevation-strategy=passwordless if remote has NOPASSWD \
           configured."
                )
            }
            // run0 passwordless: use --no-ask-password flag
            ("run0", ElevationStrategy::Passwordless) => {
                Ok(format!("run0 --no-ask-password {base_cmd}"))
            }
            ("run0", _) => {
                bail!(
                    "run0 does not support stdin password input for remote deployment. \
           Use --elevation-strategy=passwordless if authentication is not \
           required."
                )
            }
            // pkexec: no passwordless support
            ("pkexec", _) => {
                bail!(
                    "pkexec does not support non-interactive password input for remote \
           deployment. pkexec requires a polkit agent which is not available \
           over SSH."
                )
            }
            // Unknown program: bail instead of guessing
            (_, ElevationStrategy::Passwordless) => {
                bail!(
                    "Unknown elevation program '{}' does not have known passwordless \
           support. Only sudo, doas, and run0 are supported with \
           --elevation-strategy=passwordless",
                    program_name
                )
            }
            (..) => {
                bail!(
                    "Unknown elevation program '{}' does not support stdin password \
           input for remote deployment. Only sudo supports password input \
           over SSH. Use --elevation-strategy=passwordless if remote has \
           passwordless elevation configured, or use a known elevation \
           program (sudo/doas/run0).",
                    program_name
                )
            }
        }
    } else {
        Ok(base_cmd.to_owned())
    }
}

fn remote_sudo_command(
    prefix: &str,
    base_cmd: &str,
    sudo_config: &SudoConfig,
) -> String {
    let sudo_opts = sudo_config
        .opts
        .iter()
        .map(|opt| shell_quote(opt))
        .collect::<Vec<_>>()
        .join(" ");

    if sudo_opts.is_empty() {
        format!("sudo {prefix} {base_cmd}")
    } else {
        format!("sudo {prefix} {sudo_opts} {base_cmd}")
    }
}

fn nixos_activation_command(
    switch_to_config: &str,
    action: &str,
    install_bootloader: bool,
    ssh_config: &SshConfig,
) -> String {
    let mut parts = Vec::new();

    if install_bootloader {
        parts.push("NIXOS_INSTALL_BOOTLOADER=1".to_owned());
    }

    if let Some(no_check) = &ssh_config.nixos_no_check {
        parts.push(format!("NIXOS_NO_CHECK={}", shell_quote(no_check)));
    }

    parts.push(format!("{} {action}", shell_quote(switch_to_config)));
    parts.join(" ")
}

/// Register a SIGINT handler that sets the global interrupt flag.
///
/// This function is idempotent - multiple calls are safe and will not
/// create multiple handlers. Uses `signal_hook::flag::register` which
/// is async-signal-safe.
///
/// # Errors
///
/// Returns an error if the signal handler cannot be registered.
fn register_interrupt_handler() -> Result<()> {
    use signal_hook::{consts::SIGINT, flag};

    if HANDLER_REGISTERED.get().is_some() {
        return Ok(());
    }

    // Not registered yet, register it
    flag::register(SIGINT, Arc::clone(get_interrupt_flag()))
        .context("Failed to register SIGINT handler")?;

    // Mark as registered
    // The race condition here is benign. Worst case, we register twice, but both
    // handlers will set the same flag which is fine
    _ = HANDLER_REGISTERED.set(());

    Ok(())
}

/// Guard that cleans up SSH `ControlMaster` sockets on drop.
///
/// This ensures SSH control connections are properly closed when remote
/// operations complete, preventing lingering SSH processes.
#[must_use]
pub struct SshControlGuard {
    control_dir: PathBuf,
}

impl Drop for SshControlGuard {
    fn drop(&mut self) {
        cleanup_ssh_control_sockets(&self.control_dir);
        if let Err(error) = std::fs::remove_dir(&self.control_dir) {
            debug!(
                "Could not remove SSH control directory {}: {error}",
                self.control_dir.display()
            );
        }
    }
}

/// Clean up SSH `ControlMaster` sockets in the control directory.
///
/// Iterates through all ssh-* control sockets and sends the "exit" command
/// to close the master connection. Errors are logged but not propagated.
fn cleanup_ssh_control_sockets(control_dir: &std::path::Path) {
    debug!(
        "Cleaning up SSH control sockets in {}",
        control_dir.display()
    );

    // Read directory entries
    let entries = match std::fs::read_dir(control_dir) {
        Ok(entries) => entries,
        Err(err) => {
            // Directory might not exist if no SSH connections were made
            debug!(
                "Could not read SSH control directory {}: {}",
                control_dir.display(),
                err
            );
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // Only process files starting with "ssh-"
        if let Some(filename) = path.file_name().and_then(|n| n.to_str())
            && filename.starts_with("ssh-")
        {
            debug!("Closing SSH control socket: {}", path.display());

            // Run: ssh -o ControlPath=<socket> -O exit dummyhost
            let result = Exec::cmd("ssh")
                .args(["-o", &format!("ControlPath={}", path.display())])
                .args(["-O", "exit", "dummyhost"])
                .stdout(Redirection::Pipe)
                .stderr(Redirection::Pipe)
                .capture();

            match result {
                Ok(capture) => {
                    if !capture.exit_status.success() {
                        // This is normal if the connection was already closed
                        debug!(
                            "SSH control socket cleanup exited with status {:?} for {}",
                            capture.exit_status,
                            path.display()
                        );
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        "Failed to close SSH control socket at {}: {}",
                        path.display(),
                        err
                    );
                }
            }
        }
    }
}

/// Initialize SSH control socket management.
///
/// Returns a guard that closes control connections and removes the socket
/// directory when dropped.
///
/// # Errors
///
/// Returns an error if the configured socket directory cannot be created.
pub fn init_ssh_control(config: &SshConfig) -> Result<SshControlGuard> {
    std::fs::create_dir_all(&config.control_dir).context_with(|| {
        format!(
            "Failed to create SSH control directory {}",
            config.control_dir.display()
        )
    })?;
    Ok(SshControlGuard {
        control_dir: config.control_dir.clone(),
    })
}

/// Pre-establish a `ControlMaster` SSH connection to `host`.
///
/// This runs `ssh -T <opts> <host> true` using nh's own SSH argument
/// construction (options strictly before the hostname),  and thus creating a
/// multiplexed master connection in the control socket directory.
///
/// Subsequent SSH invocations that delegate to Nix internals (e.g. `nix copy`
/// with an SSH store URI) will reuse this already-authenticated socket via
/// `ControlMaster=auto`, even if those internals would otherwise pass SSH flags
/// in the wrong order.
///
/// Must be called after [`init_ssh_control`] so that the control directory
/// exists.
///
/// # Errors
///
/// Returns an error if the SSH connection cannot be established.
pub fn open_ssh_control_master(
    host: &Host,
    ssh_config: &SshConfig,
) -> Result<()> {
    let ssh_opts = get_ssh_opts(ssh_config);
    debug!("Establishing SSH ControlMaster to '{host}'");

    let mut cmd = Exec::cmd("ssh");
    for opt in &ssh_opts {
        cmd = cmd.arg(opt);
    }
    cmd = cmd.arg("-T").arg(host.ssh_host()).arg("true");

    let capture = cmd.capture().context_with(|| {
        format!("Failed to connect to remote host '{host}'")
    })?;

    if !capture.exit_status.success() {
        bail!(
            "SSH connection to '{}' failed:\n{}",
            host,
            capture.stderr_str().trim()
        );
    }

    Ok(())
}

/// Probe the effective uid on the remote host after SSH login.
///
/// This runs `id -u` over the already-opened `ControlMaster` connection and
/// returns the parsed value. Callers use this to decide whether elevation is
/// needed without trusting the username convention.
///
/// This must be called after [`open_ssh_control_master`] so the multiplexed
/// connection is already up.
///
/// # Errors
///
/// Returns an error if the SSH connection fails, `id -u` exits non-zero, or
/// its stdout cannot be parsed as a `u32`.
pub fn probe_remote_uid(
    host: &Host,
    ssh_config: &SshConfig,
) -> Result<u32> {
    let ssh_opts = get_ssh_opts(ssh_config);
    let mut cmd = Exec::cmd("ssh");
    for opt in &ssh_opts {
        cmd = cmd.arg(opt);
    }
    cmd = cmd.arg("-T").arg(host.ssh_host()).arg("id -u");

    let capture = cmd.capture().context_with(|| {
        format!("Failed to probe remote uid on '{host}'")
    })?;

    if !capture.exit_status.success() {
        bail!(
            "Remote uid probe on '{}' failed:\n{}",
            host,
            capture.stderr_str().trim()
        );
    }

    capture
        .stdout_str()
        .trim()
        .parse::<u32>()
        .context_with(|| {
            format!("Unexpected `id -u` output from '{host}'")
        })
        .map_err(Into::into)
}

/// A parsed remote host specification.
///
/// Handles various formats:
///
/// - `hostname`
/// - `user@hostname`
/// - `ssh://[user@]hostname` (scheme preserved for Nix store commands)
/// - `ssh-ng://[user@]hostname` (scheme preserved for Nix store commands)
#[derive(Eq, PartialEq, Debug, Clone, Copy)]
enum NixStoreScheme {
    Ssh,
    SshNg,
}

impl NixStoreScheme {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::SshNg => "ssh-ng",
        }
    }
}

// NOTE: This _deliberately_ does not implement `Eq` or `PartialEq` because we
// need to be clear about what we want to compare; This might differ on a
// case-to-case basis. By not implementing `Eq`, we avoid accidental misuse of
// equality comparisons that might conflate different concepts, such as
// cases where we only want to compare the host and do not care about the
// scheme.
#[derive(Debug, Clone)]
pub struct Host {
    /// The host string (may include user@).
    host: String,
    store_scheme: NixStoreScheme,
}

impl Host {
    /// Get the hostname part without the `user@` prefix.
    ///
    /// Used for hostname comparisons when determining if two `RemoteHost`
    /// instances refer to the same physical host (e.g., detecting when
    /// `build_host` == `target_host` regardless of different user credentials).
    ///
    /// Returns the bracketed IPv6 address as-is if present (e.g.,
    /// `[2001:db8::1]`).
    ///
    /// # Panics
    ///
    /// This function will never panic in practice because `rsplit('@').next()`
    /// always returns at least one element (the original string if no '@'
    /// exists).
    #[must_use]
    pub fn hostname(&self) -> &str {
        #[expect(
            clippy::unwrap_used,
            reason = "`rsplit('@')` always yields at least one element"
        )]
        self.host.rsplit('@').next().unwrap()
    }

    /// Parse a host specification string.
    ///
    /// Accepts:
    /// - `hostname`
    /// - `user@hostname`
    /// - `ssh://[user@]hostname`
    /// - `ssh-ng://[user@]hostname`
    ///
    /// URI schemes are stripped for raw SSH calls and preserved when constructing
    /// Nix store URIs. Bare hosts default to `ssh-ng://`.
    ///
    /// # Errors
    ///
    /// Returns an error if the host specification is invalid (empty hostname,
    /// empty username, contains invalid characters like `:` or `/`).
    pub fn parse(input: &str) -> Result<Self> {
        let (host, store_scheme) =
            input.strip_prefix("ssh-ng://").map_or_else(
                || {
                    input
                        .strip_prefix("ssh://")
                        .map_or((input, NixStoreScheme::SshNg), |host| {
                            (host, NixStoreScheme::Ssh)
                        })
                },
                |host| (host, NixStoreScheme::SshNg),
            );

        if host.is_empty() {
            bail!("Empty hostname in host specification");
        }

        // Validate: check for empty user in user@host format
        if host.starts_with('@') {
            bail!("Empty username in host specification: {input}");
        }
        if host.ends_with('@') {
            bail!("Empty hostname in host specification: {input}");
        }

        // Validate hostname doesn't contain invalid characters
        // (after stripping any user@ prefix for the check)
        let hostname_part = host.rsplit('@').next().unwrap_or(host);
        if hostname_part.contains('/') {
            bail!(
                "Invalid hostname '{hostname_part}': contains '/'. Did you mean to \
         use a bare hostname?"
            );
        }

        // Check for colons, but allow them in bracketed IPv6 addresses
        if hostname_part.contains(':') {
            // Check if this is a bracketed IPv6 address
            let is_bracketed_ipv6 = hostname_part.starts_with('[')
                && hostname_part.contains(']');

            if !is_bracketed_ipv6 {
                bail!(
                    "Invalid hostname '{}': contains ':'. Ports should be specified via \
           NIX_SSHOPTS=\"-p 2222\" or ~/.ssh/config",
                    hostname_part
                );
            }

            // Validate bracket matching for IPv6
            if !hostname_part.ends_with(']') {
                bail!(
                    "Invalid IPv6 address '{}': contains characters after closing \
           bracket",
                    hostname_part
                );
            }

            let open_count = hostname_part.matches('[').count();
            let close_count = hostname_part.matches(']').count();
            if open_count != 1 || close_count != 1 {
                bail!(
                    "Invalid IPv6 address '{}': mismatched brackets",
                    hostname_part
                );
            }
        }

        Ok(Self {
            host: host.to_owned(),
            store_scheme,
        })
    }

    /// Get the SSH-compatible host string.
    ///
    /// Strips brackets from IPv6 addresses since SSH doesn't accept them.
    /// Preserves zone IDs (`%eth0`) and `user@` prefix if present.
    ///
    /// Examples:
    ///
    /// - `[2001:db8::1]` -> `2001:db8::1`
    /// - `user@[2001:db8::1]` -> `user@2001:db8::1`
    /// - `[fe80::1%eth0]` -> `fe80::1%eth0`
    /// - `host.example` -> `host.example`
    #[must_use]
    pub fn ssh_host(&self) -> String {
        let hostname = self.hostname();

        // Check for bracketed IPv6 address
        if let Some(inner) = hostname
            .strip_prefix('[')
            .and_then(|hostname| hostname.strip_suffix(']'))
        {
            // Validate it's actually a valid IPv6 address
            // Split on '%' to validate only the address part (zone ID is
            // SSH-specific)
            let addr_part = inner.split('%').next().unwrap_or(inner);
            if addr_part.parse::<std::net::Ipv6Addr>().is_ok() {
                // Reconstruct with user@ prefix if present
                if let Some((user, _hostname)) = self.host.split_once('@')
                {
                    return format!("{user}@{inner}");
                }
                return inner.to_owned();
            }
        }

        // Not IPv6 or not bracketed, return as-is
        self.host.clone()
    }

    /// Get the SSH store URI used by Nix store commands.
    #[must_use]
    pub fn nix_store_uri(&self) -> String {
        format!("{}://{}", self.store_scheme.as_str(), self.host)
    }
}

impl std::str::FromStr for Host {
    type Err = Report;

    fn from_str(input: &str) -> Result<Self> {
        Self::parse(input)
    }
}

impl std::fmt::Display for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.host)
    }
}

/// Get the default SSH options for connection multiplexing.
/// Includes a `ControlPath` pointing to our control socket directory.
fn get_default_ssh_opts(config: &SshConfig) -> Vec<String> {
    let control_path = config.control_dir.join("ssh-%n");

    vec![
        "-o".to_owned(),
        "ControlMaster=auto".to_owned(),
        "-o".to_owned(),
        format!("ControlPath={}", control_path.display()),
        "-o".to_owned(),
        "ControlPersist=60".to_owned(),
    ]
}

/// Shell-quote a string for safe passing through SSH to remote shell.
fn shell_quote(word: &str) -> String {
    // Use shlex::try_quote for battle-tested quoting
    // Returns Cow::Borrowed if no quoting needed, Cow::Owned if quoted
    shlex::try_quote(word).map_or_else(
        |_| format!("'{}'", word.replace('\'', r"'\''")),
        std::borrow::Cow::into_owned,
    )
}

/// Get SSH options from `NH_SSHOPTS` (or `NIX_SSHOPTS` for compatibility)
/// plus our defaults.
#[must_use]
pub fn get_ssh_opts(config: &SshConfig) -> Vec<String> {
    let mut opts = config.user_opts.clone();
    opts.extend(get_default_ssh_opts(config));
    opts
}

/// Get SSH options as a string suitable for the `NIX_SSHOPTS` environment
/// variable passed to Nix store commands. Reads user opts from `SshConfig`
/// and appends our defaults.
fn get_nix_sshopts_env(config: &SshConfig) -> String {
    let default_opts = get_default_ssh_opts(config);

    if config.user_opts.is_empty() {
        default_opts.join(" ")
    } else {
        format!(
            "{} {}",
            config.user_opts.join(" "),
            default_opts.join(" ")
        )
    }
}

/// Check if remote cleanup is enabled.
///
/// Returns the `cleanup_remote` field from `SshConfig`.
fn should_cleanup_remote(config: &SshConfig) -> bool {
    config.cleanup_remote
}

fn kill_and_wait(job: &Job) -> io::Result<()> {
    let kill_result = job.kill();
    job.wait()?;
    kill_result
}

/// Attempt to clean up a remote process using pkill.
///
/// This is a best-effort (and opt-in) operation called when the user interrupts
/// a remote build. It tries to terminate the remote nix process via SSH and
/// pkill, but is inherently fragile due to the nature of remote building
/// semantics.
///
/// # Arguments
///
/// * `host` - The remote host where the process is running
/// * `remote_cmd` - The original command that was run remotely, used for pkill
///   matching
fn attempt_remote_cleanup(
    host: &Host,
    remote_cmd: &str,
    ssh_config: &SshConfig,
) {
    if !should_cleanup_remote(ssh_config) {
        return;
    }

    let ssh_opts = get_ssh_opts(ssh_config);
    let quoted_cmd = shell_quote(remote_cmd); // for safe passing through pkill's --full argument

    // Build the pkill command:
    // pkill -INT --full '<quoted_cmd>' will match the exact command line
    let pkill_cmd = format!("pkill -INT --full {quoted_cmd}");

    // Build SSH command with stderr capture for diagnostics
    let mut ssh_cmd = Exec::cmd("ssh").stderr(Redirection::Pipe);
    for opt in &ssh_opts {
        ssh_cmd = ssh_cmd.arg(opt);
    }
    ssh_cmd = ssh_cmd.arg(host.ssh_host()).arg(&pkill_cmd);

    debug!(
        "Attempting remote cleanup on '{host}': pkill -INT --full <command>"
    );

    // Use popen with timeout to avoid hanging on unresponsive hosts
    let mut job = match ssh_cmd.start() {
        Ok(process) => process,
        Err(err) => {
            info!("Failed to execute remote cleanup on '{host}': {err}");
            return;
        }
    };

    // Wait up to 5 seconds for cleanup to complete
    let timeout = Duration::from_secs(5);
    let exit_status = match job.wait_timeout(timeout) {
        Ok(Some(exit_status)) => exit_status,
        Ok(None) => {
            if let Err(error) = kill_and_wait(&job) {
                info!(
                    "Failed to stop timed-out remote cleanup on '{host}': {error}"
                );
            }
            info!("Remote cleanup on '{host}' timed out after 5 seconds");
            return;
        }
        Err(error) => {
            info!("Error waiting for remote cleanup on '{host}': {error}");
            return;
        }
    };

    // Check exit status
    if exit_status.success() {
        info!("Cleaned up remote process on '{}'", host);
    } else {
        // Capture stderr for error diagnosis
        let stderr =
            job.stderr.take().map_or_else(String::new, |stderr| {
                io::read_to_string(stderr).unwrap_or_else(|error| {
                    info!(
                        "Failed to read remote cleanup stderr on '{host}': {error}"
                    );
                    String::new()
                })
            });
        let stderr_lower = stderr.to_lowercase();

        if stderr.contains("No matching processes")
            || stderr_lower.contains("0 processes")
        {
            debug!(
                "No matching process found on '{host}' during cleanup (may have \
           already exited)"
            );
        } else if stderr_lower.contains("not found")
            || stderr_lower.contains("command not found")
        {
            info!(
                "pkill not available on '{}', skipping remote cleanup",
                host
            );
        } else if stderr_lower.contains("permission denied")
            || stderr_lower.contains("operation not permitted")
        {
            info!(
                "Permission denied for pkill on '{host}', skipping remote cleanup",
            );
        } else {
            info!(
                "Remote cleanup on '{host}' returned non-zero exit status"
            );
        }
    }
}

/// Get the flake experimental feature flags required for `nix` commands.
///
/// Returns `["--extra-experimental-features", "nix-command flakes"]`.
///
/// Technically this is inconsistent with our default behaviour, which is to
/// *warn* on missing features but since this is for *remote deployment* it is
/// safer to assist the user instead. Without those features, remote deployment
/// may never succeed.
fn get_flake_flags() -> Vec<&'static str> {
    vec!["--extra-experimental-features", "nix-command flakes"]
}

/// Convert `OsString` arguments to UTF-8 Strings.
///
/// Returns an error if any argument is not valid UTF-8.
fn convert_extra_args(extra_args: &[OsString]) -> Result<Vec<String>> {
    extra_args
        .iter()
        .map(|arg| {
            arg.to_str().map(String::from).ok_or_else(|| {
                report!("Extra argument is not valid UTF-8: {:?}", arg)
            })
        })
        .collect::<Result<Vec<_>>>()
}

fn nix_argv_to_strings(command: &NixCommand) -> Result<Vec<String>> {
    command
        .argv()
        .into_iter()
        .map(|arg| {
            arg.into_string().map_err(|arg| {
                report!("Nix argument is not valid UTF-8: {:?}", arg)
            })
        })
        .collect()
}

/// Run a command on a remote host via SSH.
fn run_remote_command(
    host: &Host,
    args: &[&str],
    capture_stdout: bool,
    ssh_config: &SshConfig,
) -> Result<Option<String>> {
    let ssh_opts = get_ssh_opts(ssh_config);

    debug!("Running remote command on {}: {}", host, args.join(" "));

    let quoted_args: Vec<String> =
        args.iter().map(|arg| shell_quote(arg)).collect();
    let remote_cmd = quoted_args.join(" ");
    let mut cmd = Exec::cmd("ssh");
    for opt in &ssh_opts {
        cmd = cmd.arg(opt);
    }
    cmd = cmd.arg(host.ssh_host()).arg(&remote_cmd);

    if capture_stdout {
        cmd = cmd.stdout(Redirection::Pipe).stderr(Redirection::Pipe);
    }

    let capture = cmd.capture().context_with(|| {
        format!("Failed to execute command on remote host '{host}'")
    })?;

    if !capture.exit_status.success() {
        let stderr = capture.stderr_str();
        bail!(
            "Remote command failed on '{}' (exit {:?}):\n{}",
            host,
            capture.exit_status,
            stderr
        );
    }

    if capture_stdout {
        Ok(Some(capture.stdout_str().trim().to_owned()))
    } else {
        Ok(None)
    }
}

/// Validates that essential files exist in a closure on a remote host.
///
/// Performs batched SSH checks using connection multiplexing. This is useful
/// for validating that a system closure contains all necessary files before
/// attempting activation.
///
/// # Arguments
///
/// * `host` - The remote host to check files on
/// * `closure_path` - The base path to the closure (e.g.,
///   `/nix/store/xxx-nixos-system`)
/// * `essential_files` - Slice of (`relative_path`, `description`) tuples for
///   files to validate
/// * `context_info` - Optional context for error messages (e.g., "built on
///   'host1'")
///
/// # Returns
///
/// Returns `Ok(())` if all files exist, or an error describing which files are
/// missing.
///
/// # Errors
///
/// Returns an error if:
///
/// - SSH connection to the remote host fails
/// - Any of the essential files are missing
pub fn validate_remote_closure(
    host: &Host,
    closure_path: &Path,
    essential_files: &[(&str, &str)],
    context_info: Option<&str>,
    ssh_config: &SshConfig,
) -> Result<()> {
    let ssh_opts = get_ssh_opts(ssh_config);

    let mut missing = Vec::new();
    let mut ssh_stderr = String::new();

    for (file, description) in essential_files {
        let remote_path = closure_path.join(file);
        let path_str = remote_path.to_str().ok_or_else(|| {
            report!("Path is not valid UTF-8: {}", remote_path.display())
        })?;
        let quoted_path = shlex::try_quote(path_str).map_err(|err| {
            report!(
                "Failed to quote path for shell: {}: {err}",
                remote_path.display()
            )
        })?;
        let test_cmd = format!("test -e {quoted_path}");

        let check_result = std::process::Command::new("ssh")
            .args(&ssh_opts)
            .arg(host.ssh_host())
            .arg(&test_cmd)
            .output();

        match check_result {
            Ok(output) if !output.status.success() => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.is_empty() {
                    ssh_stderr = stderr.to_string();
                    break;
                }
                missing.push(format!("  - {file} ({description})"));
            }
            Ok(_) => {} // File exists
            Err(err) => {
                bail!(
                    "Failed to check file existence on remote host {}: {}",
                    host,
                    err
                )
            }
        }
    }

    if !ssh_stderr.trim().is_empty() {
        let host_context = context_info.map_or_else(
            || format!("on remote host '{host}'"),
            |ctx| format!("on remote host '{host}' ({ctx})"),
        );

        return Err(report!(
            "Command execution failed {}: {}",
            host_context,
            ssh_stderr.trim()
        ));
    }

    if !missing.is_empty() {
        let missing_list = missing.join("\n");

        // Build context-aware error message
        let host_context = context_info.map_or_else(
            || format!("on remote host '{host}'"),
            |ctx| format!("on remote host '{host}' ({ctx})"),
        );

        return Err(report!(
            "Closure validation failed {}.\n\nMissing essential files in store path \
       '{}':\n{}\n\nThis typically happens when:\n1. Required system \
       components are disabled in your configuration\n2. The build was \
       incomplete or corrupted\n3. The Nix store path was not fully copied to \
       the target host\n\nTo fix this:\n1. Verify your configuration enables \
       all required components\n2. Ensure the complete closure was copied: \
       nix copy --to {} {}\n3. Rebuild your configuration if the problem \
       persists\n4. Use --no-validate to bypass this check if you're certain \
       the system is correctly configured",
            host_context,
            closure_path.display(),
            missing_list,
            host.nix_store_uri(),
            closure_path.display()
        ));
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Represents the type of activation to perform on a remote system.
///
/// This determines which action the system's activation script will execute.
pub enum ActivationType {
    /// Run the configuration in a test mode without activating.
    Test,

    /// Atomically switch to the new configuration.
    Switch,

    /// Make the new configuration the default boot option.
    Boot,
}

impl ActivationType {
    /// Get the string representation used by activation scripts.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Switch => "switch",
            Self::Boot => "boot",
        }
    }
}

/// Represents the target platform for remote operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// NixOS system configuration.
    NixOS,
}

/// Configuration for remote activation operations.
#[derive(Debug)]
pub struct ActivateRemoteConfig {
    /// The target platform for activation.
    pub platform: Platform,

    /// The type of activation to perform.
    pub activation_type: ActivationType,

    /// Whether to install the bootloader during activation.
    pub install_bootloader: bool,

    /// Whether to show output logs during activation.
    pub show_logs: bool,

    /// Elevation strategy for remote activation commands.
    ///
    /// - `None`: No elevation, run commands as the remote user
    /// - `Some(strategy)`: Use the specified elevation strategy (sudo, doas,
    ///   etc.)
    pub elevation: Option<ElevationStrategy>,
}

/// Activate a system configuration on a remote host.
///
/// Currently only supports NixOS.
///
/// # Arguments
///
/// * `host` - The remote host to activate on
/// * `system_profile` - The path to the NixOS system profile (e.g.,
///   /nix/var/nix/profiles/system)
/// * `config` - Activation configuration options
///
/// # Errors
///
#[expect(clippy::module_name_repetitions, reason = "It's clearer")]
pub fn activate_remote(
    host: &Host,
    system_profile: &Path,
    config: &ActivateRemoteConfig,
    runtime_env: &RuntimeEnv,
    ssh_config: &SshConfig,
    sudo_config: &SudoConfig,
) -> Result<()> {
    match config.platform {
        Platform::NixOS => activate_nixos_remote(
            host,
            system_profile,
            config,
            runtime_env,
            ssh_config,
            sudo_config,
        ),
    }
}

/// Activate a NixOS system configuration on a remote host.
///
/// Handles the SSH commands required to activate a NixOS system. Supports
/// test, switch, and boot activation types.
///
/// # Arguments
///
/// * `host` - The remote host to activate on
/// * `system_profile` - The path to the NixOS system profile
/// * `config` - Activation configuration options
///
/// # Errors
///
fn activate_nixos_remote(
    host: &Host,
    system_profile: &Path,
    config: &ActivateRemoteConfig,
    runtime_env: &RuntimeEnv,
    ssh_config: &SshConfig,
    sudo_config: &SudoConfig,
) -> Result<()> {
    let ssh_opts = get_ssh_opts(ssh_config);

    // Prompt for password if elevation is needed
    // Skip for None (no elevation) and Passwordless (remote has NOPASSWD
    // configured)
    let sudo_password = if let Some(strategy) = &config.elevation {
        if matches!(
            strategy,
            ElevationStrategy::None | ElevationStrategy::Passwordless
        ) {
            // None: no elevation program used
            // Passwordless: elevation program used but no password needed
            None
        } else {
            let host_str = host.ssh_host();
            if let Some(cached_password) = get_cached_password(&host_str)?
            {
                Some(cached_password)
            } else {
                let password = inquire::Password::new(&format!(
                    "[sudo] password for {host_str}:"
                ))
                .without_confirmation()
                .prompt()
                .context("Failed to read sudo password")?;
                if password.is_empty() {
                    bail!("Password cannot be empty");
                }
                let secret_password = SecretString::new(password.into());
                cache_password(&host_str, secret_password.clone())?;
                Some(secret_password)
            }
        }
    } else {
        None
    };

    let switch_to_config =
        system_profile.join("bin/switch-to-configuration");

    let switch_path_str = switch_to_config.to_str().ok_or_else(|| {
        report!("switch-to-configuration path contains invalid UTF-8")
    })?;

    match config.activation_type {
        ActivationType::Test | ActivationType::Switch => {
            let action = config.activation_type.as_str();

            let mut ssh_cmd = Exec::cmd("ssh");
            for opt in &ssh_opts {
                ssh_cmd = ssh_cmd.arg(opt);
            }
            // Add -T flag to disable pseudo-terminal allocation (needed for stdin)
            ssh_cmd = ssh_cmd.arg("-T");
            ssh_cmd = ssh_cmd.arg(host.ssh_host());

            // Build the remote command using helper function
            let base_cmd = nixos_activation_command(
                switch_path_str,
                action,
                false,
                ssh_config,
            );
            let remote_cmd = build_remote_command(
                config.elevation.as_ref(),
                &base_cmd,
                runtime_env,
                sudo_config,
            )?;

            ssh_cmd = ssh_cmd.arg(remote_cmd);

            // Pass password via stdin if elevation is needed
            if let Some(password) = &sudo_password {
                ssh_cmd = ssh_cmd.stdin(
                    format!("{}\n", password.expose_secret()).into_bytes(),
                );
            }

            debug!(?ssh_cmd, "Activating NixOS configuration");

            let capture = ssh_cmd
                .capture()
                .context("Failed to activate NixOS configuration")?;

            if config.show_logs {
                println!("{}", capture.stdout_str());
            }

            if !capture.exit_status.success() {
                bail!(
                    "Activation ({}) failed on '{}':\n{}",
                    action,
                    host,
                    capture.stderr_str()
                );
            }
        }

        ActivationType::Boot => {
            let mut profile_ssh_cmd = Exec::cmd("ssh");
            for opt in &ssh_opts {
                profile_ssh_cmd = profile_ssh_cmd.arg(opt);
            }
            // Add -T flag to disable pseudo-terminal allocation (needed for stdin)
            profile_ssh_cmd = profile_ssh_cmd.arg("-T");
            profile_ssh_cmd = profile_ssh_cmd.arg(host.ssh_host());

            // Build the remote command using helper function
            let base_cmd = format!(
                "nix build --no-link --profile {} {}",
                NIXOS_SYSTEM_PROFILE,
                shell_quote(&system_profile.to_string_lossy())
            );
            let profile_remote_cmd = build_remote_command(
                config.elevation.as_ref(),
                &base_cmd,
                runtime_env,
                sudo_config,
            )?;

            profile_ssh_cmd = profile_ssh_cmd.arg(profile_remote_cmd);

            // Pass password via stdin if elevation is needed
            if let Some(password) = &sudo_password {
                profile_ssh_cmd = profile_ssh_cmd.stdin(
                    format!("{}\n", password.expose_secret()).into_bytes(),
                );
            }

            debug!(?profile_ssh_cmd, "Setting NixOS profile");

            let profile_capture = profile_ssh_cmd
                .capture()
                .context("Failed to set NixOS profile")?;

            if !profile_capture.exit_status.success() {
                bail!(
                    "Failed to set system profile on '{}':\n{}",
                    host,
                    profile_capture.stderr_str()
                );
            }

            let mut boot_ssh_cmd = Exec::cmd("ssh");
            for opt in &ssh_opts {
                boot_ssh_cmd = boot_ssh_cmd.arg(opt);
            }
            // Add -T flag to disable pseudo-terminal allocation (needed for stdin)
            boot_ssh_cmd = boot_ssh_cmd.arg("-T");
            boot_ssh_cmd = boot_ssh_cmd.arg(host.ssh_host());

            // Build the remote command using helper function
            let boot_activation_cmd = nixos_activation_command(
                switch_path_str,
                "boot",
                config.install_bootloader,
                ssh_config,
            );
            let boot_remote_cmd = build_remote_command(
                config.elevation.as_ref(),
                &boot_activation_cmd,
                runtime_env,
                sudo_config,
            )?;

            boot_ssh_cmd = boot_ssh_cmd.arg(boot_remote_cmd);

            // Pass password via stdin if elevation is needed
            if let Some(password) = &sudo_password {
                boot_ssh_cmd = boot_ssh_cmd.stdin(
                    format!("{}\n", password.expose_secret()).into_bytes(),
                );
            }

            debug!(?boot_ssh_cmd, "Bootloader activation");

            let boot_capture = boot_ssh_cmd
                .capture()
                .context("Bootloader activation failed")?;

            if !boot_capture.exit_status.success() {
                bail!(
                    "Bootloader activation failed on '{}':\n{}",
                    host,
                    boot_capture.stderr_str()
                );
            }
        }
    }

    Ok(())
}

/// System profile path for NixOS.
/// Used by remote activation functions.
const NIXOS_SYSTEM_PROFILE: &str = "/nix/var/nix/profiles/system";

/// Evaluate a flake installable to get its derivation path.
/// Matches nixos-rebuild-ng: `nix eval --raw <flake>.drvPath`.
fn eval_drv_path(installable: &Installable) -> Result<PathBuf> {
    // Build the installable with .drvPath appended
    let drv_installable = match installable {
        Installable::Flake {
            reference,
            attribute,
        } => {
            let mut drv_attr = attribute.clone();
            drv_attr.push("drvPath".to_owned());
            Installable::Flake {
                reference: reference.clone(),
                attribute: drv_attr,
            }
        }
        Installable::File { path, attribute } => {
            let mut drv_attr = attribute.clone();
            drv_attr.push("drvPath".to_owned());
            Installable::File {
                path: path.clone(),
                attribute: drv_attr,
            }
        }
        Installable::Expression {
            expression,
            attribute,
        } => {
            let mut drv_attr = attribute.clone();
            drv_attr.push("drvPath".to_owned());
            Installable::Expression {
                expression: expression.clone(),
                attribute: drv_attr,
            }
        }
        Installable::Store { path } => {
            bail!(
                "Cannot perform remote build with store path '{}'. Store paths are \
         already built.",
                path.display()
            );
        }
    };

    let args = drv_installable.to_args();
    debug!("Evaluating drvPath: nix eval --raw {:?}", args);

    let cmd = NixCommand::new(CommandKind::Eval)
        .global_args(get_flake_flags())
        .arg("--raw")
        .args(&args)
        .into_exec()
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Pipe);

    let capture = cmd.capture().context("Failed to run nix eval")?;

    if !capture.exit_status.success() {
        bail!(
            "Failed to evaluate derivation path:\n{}",
            capture.stderr_str()
        );
    }

    let drv_path = PathBuf::from(capture.stdout_str().trim().to_owned());
    if !drv_path.is_file() {
        bail!(
            "nix eval returned invalid derivation path: {}",
            drv_path.display()
        );
    }

    debug!("Derivation path: {}", drv_path.display());
    Ok(drv_path)
}

/// Configuration for a remote build operation.
///
/// # Host Interaction Semantics
///
/// The behavior depends on which hosts are specified:
///
/// | `build_host` | `target_host` | Behavior |
/// |--------------|---------------|----------|
/// | Some(H2)     | None          | Build on H2, copy result to localhost |
/// | Some(H2)     | Some(H2)      | Build on H2, no copy (build host = target) |
/// | Some(H2)     | Some(H3)      | Build on H2, try direct copy to H3; if that fails, relay through localhost |
///
/// When `build_host` and `target_host` differ, the code attempts a direct
/// copy between remotes first. If this fails (common when the hosts can't
/// see each other), it falls back to relaying through localhost:
///
/// - Direct: Host2 -> Host3
/// - Fallback: Host2 -> Host1 (localhost) → Host3
///
/// If `out_link` is requested and the result is copied to localhost, the
/// symlink points at the local store path. When the build host is also the
/// target host, the result stays remote-only and local out-link creation is
/// skipped.
#[derive(Debug, Clone)]
pub struct BuildConfig {
    /// The host to build on.
    pub build_host: Host,

    /// Optional target host to copy the result to (instead of localhost).
    /// When set, copies directly from `build_host` to `target_host`.
    pub target_host: Option<Host>,

    /// Whether to use nix-output-monitor for build output.
    pub use_nom: bool,

    /// Whether to use substitutes when copying closures.
    pub use_substitutes: bool,

    /// Extra arguments to pass to the build command.
    pub extra_args: Vec<OsString>,
}

/// Perform a remote build of a flake installable.
///
/// This implements the `build_remote_flake` workflow from nixos-rebuild-ng:
/// 1. Evaluate drvPath locally via `nix eval --raw`
/// 2. Copy the derivation to the build host via `nix copy`
/// 3. Build on remote host via `nix build <drv>^* --print-out-paths`
/// 4. Copy the result back (to localhost or `target_host`)
///
/// Returns the output path in the Nix store.
///
/// # Errors
///
#[expect(clippy::module_name_repetitions, reason = "It's clearer")]
pub fn build_remote(
    installable: &Installable,
    config: &BuildConfig,
    out_link: Option<&std::path::Path>,
    ssh_config: &SshConfig,
) -> Result<PathBuf> {
    let build_host = &config.build_host;
    let use_substitutes = config.use_substitutes;

    // Step 1: Evaluate drvPath locally
    info!("Evaluating derivation path");
    let drv_path = eval_drv_path(installable)?;

    // Step 2: Copy derivation to build host
    copy_to_remote(build_host, &drv_path, use_substitutes, ssh_config)?;

    // Step 3: Build on remote
    let out_path =
        build_on_remote(build_host, &drv_path, config, ssh_config)?;

    // Step 4: Copy result to destination
    //
    // Optimizes copy paths based on hostname comparison:
    // - When build_host != target_host: copy build -> target, then build -> local
    //   if needed
    // - When build_host == target_host: skip redundant copies and leave the
    //   result remote-only
    // - When target_host is None: always copy build -> local
    let target_is_build_host = config
        .target_host
        .as_ref()
        .is_some_and(|th| th.hostname() == build_host.hostname());

    let need_local_copy = match &config.target_host {
        None => true,
        Some(_target_host) if target_is_build_host => {
            debug!(
                "Skipping copy from build host to target host (same host: {})",
                build_host.hostname()
            );

            // When build_host == target_host and both are remote, the result is
            // already where it needs to be. No need to copy to localhost even if
            // out_link is requested, since the closure will be activated remotely.
            // This is a little confusing, but frankly, respecting --out-link to
            // create a local path while everything happens remotely is a bit
            // more confusing.
            false
        }
        Some(target_host) => {
            match copy_closure_between_hosts(
                build_host,
                target_host,
                &out_path,
                use_substitutes,
                ssh_config,
            ) {
                Ok(()) => {
                    debug!(
                        "Successfully copied closure directly from {} to {}",
                        build_host.hostname(),
                        target_host.hostname()
                    );
                    out_link.is_some()
                }
                Err(err) => {
                    warn!(
                        "Direct copy from {} to {} failed: {}. Will relay through \
             localhost.",
                        build_host.hostname(),
                        target_host.hostname(),
                        err
                    );
                    true
                }
            }
        }
    };

    if need_local_copy {
        copy_closure_from(build_host, &out_path, ssh_config)?;
    }

    // Create local out-link if requested and the result is in local store
    // When build_host == target_host (both remote), skip out-link creation
    // since the closure is remote and won't be copied to localhost
    if let Some(link) = out_link {
        if need_local_copy {
            debug!(
                "Creating out-link: {} -> {}",
                link.display(),
                out_path
            );
            // Remove existing symlink/file if present
            let removal = match std::fs::remove_file(link) {
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    Ok(())
                }
                result => result,
            };
            removal.context(format!(
                "Failed to remove existing out-link {}",
                link.display()
            ))?;
            std::os::unix::fs::symlink(&out_path, link)
                .context("Failed to create out-link")?;
        } else {
            debug!(
                "Skipping out-link creation: result is on remote host and not copied \
         to localhost"
            );
        }
    }

    Ok(PathBuf::from(out_path))
}

/// Build a derivation on a remote host.
fn build_on_remote(
    host: &Host,
    drv_path: &Path,
    config: &BuildConfig,
    ssh_config: &SshConfig,
) -> Result<String> {
    // Build command: nix build <drv>^* --print-out-paths [extra_args...]
    let drv_with_outputs = format!("{}^*", drv_path.display());

    if config.use_nom {
        // Check that nom is available before attempting to use it
        which::which("nom").context(
            "nom (nix-output-monitor) is required but not found in PATH",
        )?;

        build_on_remote_with_nom(
            host,
            &drv_with_outputs,
            config,
            ssh_config,
        )
    } else {
        build_on_remote_simple(host, &drv_with_outputs, config, ssh_config)
    }
}

/// Build the argument list for remote nix build commands.
/// Returns owned strings to avoid lifetime issues with `extra_args`.
fn build_nix_command(
    drv_with_outputs: &str,
    extra_flags: &[&str],
    extra_args: &[OsString],
) -> Result<Vec<String>> {
    let extra_args_strings = convert_extra_args(extra_args)?;

    nix_argv_to_strings(
        &NixCommand::new(CommandKind::Build)
            .print_build_logs(false)
            .global_args(get_flake_flags())
            .arg(drv_with_outputs)
            .args(extra_flags)
            .args(extra_args_strings),
    )
}

fn build_on_remote_simple(
    host: &Host,
    drv_with_outputs: &str,
    config: &BuildConfig,
    ssh_config: &SshConfig,
) -> Result<String> {
    // Register interrupt handler at start
    register_interrupt_handler()?;
    let ssh_opts = get_ssh_opts(ssh_config);

    let args = build_nix_command(
        drv_with_outputs,
        &["--print-out-paths"],
        &config.extra_args,
    )?;
    let arg_refs: Vec<&str> =
        args.iter().map(std::string::String::as_str).collect();

    // Build SSH command with stdout capture
    // Quote all arguments for safe shell passing
    let quoted_args: Vec<String> =
        arg_refs.iter().map(|arg| shell_quote(arg)).collect();
    let remote_cmd = quoted_args.join(" ");

    let mut ssh_cmd = Exec::cmd("ssh");
    for opt in &ssh_opts {
        ssh_cmd = ssh_cmd.arg(opt);
    }
    ssh_cmd = ssh_cmd
        .arg(host.ssh_host())
        .arg(&remote_cmd)
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Pipe);

    // Execute with start() to get a Job handle
    let mut job = ssh_cmd.start()?;

    // Wait for completion with interrupt checking
    let exit_status = loop {
        match job.wait_timeout(std::time::Duration::from_millis(100))? {
            Some(status) => break status,
            None => {
                // Check interrupt flag while waiting
                if get_interrupt_flag().load(Ordering::Relaxed) {
                    debug!("Interrupt detected, killing SSH process");

                    let stop_result = kill_and_wait(&job);

                    // Attempt remote cleanup even if local cleanup failed.
                    attempt_remote_cleanup(host, &remote_cmd, ssh_config);
                    stop_result.context(
                        "Failed to stop interrupted SSH process",
                    )?;

                    bail!("Operation interrupted by user");
                }
            }
        }
    };

    // Check exit status
    if !exit_status.success() {
        let stderr = job
            .stderr
            .take()
            .and_then(|stderr_reader| {
                io::read_to_string(stderr_reader).ok()
            })
            .unwrap_or_else(|| String::from("(no stderr)"));
        bail!("Remote command failed: {}", stderr);
    }

    // Read stdout
    let stdout = job
        .stdout
        .take()
        .ok_or_else(|| report!("Failed to capture stdout"))?;
    let mut reader = std::io::BufReader::new(stdout);
    let mut output = String::new();
    reader.read_to_string(&mut output)?;

    // --print-out-paths may return multiple lines; take first
    let out_path = output
        .lines()
        .next()
        .ok_or_else(|| report!("Remote build returned empty output"))?
        .trim()
        .to_owned();

    debug!("Remote build output: {}", out_path);
    Ok(out_path)
}

/// Build on remote with nom - pipe through nix-output-monitor.
fn build_on_remote_with_nom(
    host: &Host,
    drv_with_outputs: &str,
    config: &BuildConfig,
    ssh_config: &SshConfig,
) -> Result<String> {
    // Register interrupt handler at start
    register_interrupt_handler()?;

    let ssh_opts = get_ssh_opts(ssh_config);

    // Build the remote command with JSON output for nom
    let remote_args = build_nix_command(
        drv_with_outputs,
        &["--log-format", "internal-json", "--verbose"],
        &config.extra_args,
    )?;
    let arg_refs: Vec<&str> = remote_args
        .iter()
        .map(std::string::String::as_str)
        .collect();

    // Build SSH command
    // Quote all arguments for safe shell passing
    let quoted_remote: Vec<String> =
        arg_refs.iter().map(|arg| shell_quote(arg)).collect();
    let remote_cmd = quoted_remote.join(" ");

    let mut ssh_cmd = Exec::cmd("ssh");
    for opt in &ssh_opts {
        ssh_cmd = ssh_cmd.arg(opt);
    }
    ssh_cmd = ssh_cmd
        .arg(host.ssh_host())
        .arg(&remote_cmd)
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Merge);

    // Pipe through nom
    let nom_cmd = Exec::cmd("nom").arg("--json");
    let pipeline = (ssh_cmd | nom_cmd).stdout(Redirection::None);

    debug!(?pipeline, "Running remote build with nom");

    // Use popen() to get access to individual processes so we can check
    // ssh's exit status, not nom's. The pipeline's join() only returns
    // the exit status of the last command (nom), which always succeeds
    // even when the remote nix command fails.
    let job = pipeline.start().context("Remote build with nom failed")?;

    // Use wait_timeout in a polling loop to check interrupt flag every 100ms
    let poll_interval = Duration::from_millis(100);

    for proc in &job.processes {
        #[expect(
            clippy::needless_continue,
            reason = "Better for explicitness and consistency"
        )]
        loop {
            // Check interrupt flag before waiting
            if get_interrupt_flag().load(Ordering::Relaxed) {
                debug!("Interrupt detected during build with nom");
                // Kill remaining local processes. This will cause SSH to terminate
                // the remote command automatically.
                let stop_result = kill_and_wait(&job);

                // Attempt remote cleanup even if local cleanup failed.
                attempt_remote_cleanup(host, &remote_cmd, ssh_config);
                stop_result.context(
                    "Failed to stop interrupted remote build processes",
                )?;

                bail!("Operation interrupted by user");
            }

            // Poll process with timeout
            match proc.wait_timeout(poll_interval)? {
                Some(_) => {
                    // Process has exited, exit status is automatically cached in the
                    // Process handle. Move to next process.
                    break;
                }

                None => {
                    // Timeout elapsed, process still running - loop continues
                    // and will check interrupt flag again
                    continue;
                }
            }
        }
    }

    // Check the exit status of the FIRST process (ssh -> nix build)
    // This is the one that matters. If the remote build fails, we should fail
    // too
    if let Some(ssh_proc) = job.processes.first() {
        let exit_status = ssh_proc.wait()?;
        if !exit_status.success() {
            bail!("Remote build failed with exit status: {exit_status:?}");
        }
    }

    // nom consumed the output, so we need to query the output path separately
    // Run nix build again with --print-out-paths (it will be a no-op since
    // already built)
    let query_args =
        build_nix_command(drv_with_outputs, &["--print-out-paths"], &[])?;
    let query_refs: Vec<&str> =
        query_args.iter().map(std::string::String::as_str).collect();

    let result = run_remote_command(host, &query_refs, true, ssh_config);
    if get_interrupt_flag().load(Ordering::Relaxed) {
        debug!("Interrupt detected during output path query");
        bail!("Operation interrupted by user");
    }

    let result = result?
        .ok_or_else(|| report!("Failed to get output path after build"))?;

    let out_path = result
        .lines()
        .next()
        .ok_or_else(|| report!("Output path query returned empty"))?
        .trim()
        .to_owned();

    debug!("Remote build output: {}", out_path);
    Ok(out_path)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "Fine in tests"
    )]
    use proptest::prelude::*;

    use super::*;

    proptest! {
      #[test]
      fn hostname_always_returns_suffix_after_last_at(hostname in "\\PC*") {
          let host = Host {
            host:         hostname.clone(),
            store_scheme: NixStoreScheme::SshNg,
          };
          let expected = hostname.rsplit('@').next().unwrap();
          prop_assert_eq!(host.hostname(), expected);
      }

      #[test]
      fn hostname_is_substring_of_host(hostname in "\\PC*") {
          let host = Host {
            host:         hostname.clone(),
            store_scheme: NixStoreScheme::SshNg,
          };
          prop_assert!(hostname.contains(host.hostname()));
      }

      #[test]
      fn hostname_no_at_means_whole_string(hostname in "[^@]*") {
          let host = Host {
            host:         hostname.clone(),
            store_scheme: NixStoreScheme::SshNg,
          };
          prop_assert_eq!(host.hostname(), hostname);
      }

      #[test]
      fn hostname_with_user(user in "[a-zA-Z0-9_]+", hostname in "[a-zA-Z0-9_.-]+") {
          let full = format!("{user}@{hostname}");
          let host = Host {
            host: full,
            store_scheme: NixStoreScheme::SshNg,
          };
          prop_assert_eq!(host.hostname(), hostname);
      }

      #[test]
      fn parse_valid_bare_hostname(hostname in "[a-zA-Z0-9_.-]+") {
          let result = Host::parse(&hostname);
          prop_assert!(result.is_ok());
          let host = result.unwrap();
          prop_assert_eq!(host.hostname(), hostname);
      }

      #[test]
      fn parse_valid_user_at_hostname(user in "[a-zA-Z0-9_]+", hostname in "[a-zA-Z0-9_.-]+") {
          let full = format!("{user}@{hostname}");
          let result = Host::parse(&full);
          prop_assert!(result.is_ok());
          let host = result.unwrap();
          prop_assert_eq!(host.hostname(), hostname);
      }
    }

    #[test]
    fn parse_bare_hostname() {
        let host = Host::parse("buildserver").expect("should parse");
        assert_eq!(host.to_string(), "buildserver");
    }

    #[test]
    fn parse_user_at_hostname() {
        let host = Host::parse("root@buildserver").expect("should parse");
        assert_eq!(host.to_string(), "root@buildserver");
    }

    #[test]
    fn parse_ssh_uri_preserves_store_scheme() {
        let host = Host::parse("ssh://buildserver").expect("should parse");
        assert_eq!(host.to_string(), "buildserver");
        assert_eq!(host.nix_store_uri(), "ssh://buildserver");
    }

    #[test]
    fn parse_ssh_ng_uri_stripped() {
        let host =
            Host::parse("ssh-ng://buildserver").expect("should parse");
        assert_eq!(host.to_string(), "buildserver");
    }

    #[test]
    fn parse_ssh_uri_with_user_preserves_store_scheme() {
        let host =
            Host::parse("ssh://root@buildserver").expect("should parse");
        assert_eq!(host.to_string(), "root@buildserver");
        assert_eq!(host.nix_store_uri(), "ssh://root@buildserver");
    }

    #[test]
    fn parse_ssh_ng_uri_with_user() {
        let host = Host::parse("ssh-ng://admin@buildserver")
            .expect("should parse");
        assert_eq!(host.to_string(), "admin@buildserver");
    }

    #[test]
    fn parse_empty_fails() {
        Host::parse("").unwrap_err();
    }

    #[test]
    fn parse_empty_user_fails() {
        Host::parse("@hostname").unwrap_err();
    }

    #[test]
    fn parse_empty_hostname_fails() {
        Host::parse("user@").unwrap_err();
    }

    #[test]
    fn parse_port_rejected() {
        let Err(err) = Host::parse("hostname:22") else {
            panic!("expected error for port in hostname");
        };
        assert!(err.to_string().contains("NIX_SSHOPTS"));
    }

    #[test]
    fn parse_ipv6_bracketed() {
        let host =
            Host::parse("[2001:db8::1]").expect("should parse IPv6");
        assert_eq!(host.to_string(), "[2001:db8::1]");
        assert_eq!(host.hostname(), "[2001:db8::1]");
    }

    #[test]
    fn parse_ipv6_with_user() {
        let host = Host::parse("root@[2001:db8::1]")
            .expect("should parse IPv6 with user");
        assert_eq!(host.to_string(), "root@[2001:db8::1]");
        assert_eq!(host.hostname(), "[2001:db8::1]");
    }

    #[test]
    fn parse_ipv6_with_zone_id() {
        let host = Host::parse("[fe80::1%eth0]")
            .expect("should parse IPv6 with zone");
        assert_eq!(host.to_string(), "[fe80::1%eth0]");
    }

    #[test]
    fn parse_ipv6_ssh_ng_uri() {
        let host = Host::parse("ssh-ng://[2001:db8::1]")
            .expect("should parse IPv6 SSH-NG URI");
        assert_eq!(host.to_string(), "[2001:db8::1]");
    }

    #[test]
    fn parse_ipv6_ssh_ng_uri_with_user() {
        let host = Host::parse("ssh-ng://root@[2001:db8::1]")
            .expect("should parse IPv6 SSH-NG URI with user");
        assert_eq!(host.to_string(), "root@[2001:db8::1]");
    }

    #[test]
    fn parse_ipv6_ssh_uri_preserves_store_scheme() {
        let host = Host::parse("ssh://[2001:db8::1]")
            .expect("should parse IPv6 SSH URI");
        assert_eq!(host.to_string(), "[2001:db8::1]");
        assert_eq!(host.nix_store_uri(), "ssh://[2001:db8::1]");
    }

    #[test]
    fn parse_ipv6_ssh_uri_with_user_preserves_store_scheme() {
        let host = Host::parse("ssh://root@[2001:db8::1]")
            .expect("should parse IPv6 SSH URI with user");
        assert_eq!(host.to_string(), "root@[2001:db8::1]");
        assert_eq!(host.nix_store_uri(), "ssh://root@[2001:db8::1]");
    }

    #[test]
    fn parse_ipv6_localhost() {
        let host =
            Host::parse("[::1]").expect("should parse IPv6 localhost");
        assert_eq!(host.to_string(), "[::1]");
    }

    #[test]
    fn parse_ipv6_compressed() {
        let host = Host::parse("[2001:db8::]")
            .expect("should parse compressed IPv6");
        assert_eq!(host.to_string(), "[2001:db8::]");
    }

    #[test]
    fn parse_ipv6_unbracketed_rejected() {
        // Bare IPv6 without brackets should be rejected
        let result = Host::parse("2001:db8::1");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("NIX_SSHOPTS"));
    }

    #[test]
    fn parse_ipv6_mismatched_brackets_rejected() {
        Host::parse("[2001:db8::1").unwrap_err();
        Host::parse("2001:db8::1]").unwrap_err();
    }

    #[test]
    fn parse_ipv6_extra_brackets_rejected() {
        Host::parse("[[2001:db8::1]]").unwrap_err();
        Host::parse("[2001:db8::[1]]").unwrap_err();
    }

    #[test]
    fn parse_ipv6_with_port_rejected() {
        // IPv6 with port syntax should be rejected (use NIX_SSHOPTS)
        let result = Host::parse("[2001:db8::1]:22");
        result.unwrap_err();
    }

    #[test]
    fn parse_ipv6_chars_after_bracket_rejected() {
        // Characters after closing bracket should be rejected
        let result = Host::parse("[2001:db8::1]extra");
        result.unwrap_err();
    }

    #[test]
    fn parse_ipv6_at_inside_brackets_rejected() {
        // @ character inside brackets should be rejected (not valid IPv6)
        // This ensures [@2001:db8::1] and [2001@db8::1] are both rejected
        let result = Host::parse("[@2001:db8::1]");
        assert!(result.is_err(), "[@2001:db8::1] should be rejected");

        let result2 = Host::parse("[2001@db8::1]");
        assert!(result2.is_err(), "[2001@db8::1] should be rejected");
    }

    #[test]
    fn ssh_host_ipv6_strips_brackets() {
        let host =
            Host::parse("[2001:db8::1]").expect("should parse IPv6");
        assert_eq!(host.ssh_host(), "2001:db8::1");
    }

    #[test]
    fn ssh_host_ipv6_with_user() {
        let host =
            Host::parse("user@[2001:db8::1]").expect("should parse");
        assert_eq!(host.ssh_host(), "user@2001:db8::1");
    }

    #[test]
    fn ssh_host_ipv6_with_zone_id() {
        let host = Host::parse("[fe80::1%eth0]").expect("should parse");
        assert_eq!(host.ssh_host(), "fe80::1%eth0");
    }

    #[test]
    fn ssh_host_ipv6_with_zone_id_and_user() {
        let host =
            Host::parse("user@[fe80::1%eth0]").expect("should parse");
        assert_eq!(host.ssh_host(), "user@fe80::1%eth0");
    }

    #[test]
    fn ssh_host_ipv6_localhost() {
        let host = Host::parse("[::1]").expect("should parse");
        assert_eq!(host.ssh_host(), "::1");
    }

    #[test]
    fn ssh_host_non_ipv6_unchanged() {
        let host = Host::parse("host.example").expect("should parse");
        assert_eq!(host.ssh_host(), "host.example");
    }

    #[test]
    fn ssh_host_non_ipv6_with_user() {
        let host = Host::parse("user@host.example").expect("should parse");
        assert_eq!(host.ssh_host(), "user@host.example");
    }

    #[test]
    fn ssh_host_ssh_ng_uri_ipv6() {
        let host =
            Host::parse("ssh-ng://[2001:db8::1]").expect("should parse");
        assert_eq!(host.ssh_host(), "2001:db8::1");
    }

    #[test]
    fn ssh_host_ssh_ng_uri_ipv6_with_user() {
        let host = Host::parse("ssh-ng://root@[2001:db8::1]")
            .expect("should parse");
        assert_eq!(host.ssh_host(), "root@2001:db8::1");
    }

    #[test]
    fn ssh_host_ssh_uri_ipv6() {
        let host =
            Host::parse("ssh://[2001:db8::1]").expect("should parse");
        assert_eq!(host.ssh_host(), "2001:db8::1");
    }

    #[test]
    fn ssh_host_ssh_uri_ipv6_with_user() {
        let host =
            Host::parse("ssh://root@[2001:db8::1]").expect("should parse");
        assert_eq!(host.ssh_host(), "root@2001:db8::1");
    }

    #[test]
    fn nix_store_uri_defaults_bare_host_to_ssh_ng() {
        let host = Host::parse("build.example").expect("should parse");
        assert_eq!(host.nix_store_uri(), "ssh-ng://build.example");
    }

    #[test]
    fn nix_store_uri_for_user_host() {
        let host =
            Host::parse("user@build.example").expect("should parse");
        assert_eq!(host.nix_store_uri(), "ssh-ng://user@build.example");
    }

    #[test]
    fn nix_store_uri_preserves_ipv6_brackets() {
        let host = Host::parse("[2001:db8::1]").expect("should parse");
        assert_eq!(host.nix_store_uri(), "ssh-ng://[2001:db8::1]");
    }

    #[test]
    fn nix_store_uri_preserves_user_ipv6_brackets() {
        let host =
            Host::parse("user@[2001:db8::1]").expect("should parse");
        assert_eq!(host.nix_store_uri(), "ssh-ng://user@[2001:db8::1]");
    }

    #[test]
    fn shell_quote_simple() {
        assert_eq!(shell_quote("simple"), "simple");
        assert_eq!(
            shell_quote("/nix/store/abc123-foo"),
            "/nix/store/abc123-foo"
        );
    }

    #[test]
    fn ssh_configuration_is_parsed_from_the_startup_snapshot() {
        let env = RuntimeEnv::from_pairs([
            ("NH_SSHOPTS", "-p 2222 -o 'ProxyJump=bastion.example'"),
            ("NIX_SSHOPTS", "--legacy"),
            ("NH_REMOTE_CLEANUP", "0"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("SSH_AUTH_SOCK", "/tmp/ssh-agent.sock"),
            ("NIXOS_NO_CHECK", "1"),
        ]);

        let config = SshConfig::from_env(&env).unwrap();
        assert_eq!(
            config.user_opts,
            ["-p", "2222", "-o", "ProxyJump=bastion.example"]
        );
        assert!(!config.cleanup_remote);
        assert_eq!(
            config.control_dir.parent(),
            Some(Path::new("/run/user/1000"))
        );
        assert_eq!(
            config.agent_socket.as_deref(),
            Some(Path::new("/tmp/ssh-agent.sock"))
        );
        assert_eq!(config.nixos_no_check.as_deref(), Some("1"));
    }

    #[test]
    fn malformed_ssh_options_are_rejected_at_startup() {
        let env =
            RuntimeEnv::from_pairs([("NH_SSHOPTS", "'unterminated")]);

        let error = SshConfig::from_env(&env).unwrap_err();
        assert!(error.to_string().contains("NH_SSHOPTS"));
    }
}
