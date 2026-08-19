use std::{
    collections::HashMap,
    convert::Infallible,
    env,
    ffi::{OsStr, OsString},
    io::{BufRead, Write},
    path::PathBuf,
    str::FromStr,
    sync::{Mutex, OnceLock},
};

use color_eyre::{
    Result,
    eyre::{self, Context, bail},
};
use nh_installable::Installable;
pub use nix_command::{CommandKind, NixCommand, SubprocessEnv};
use secrecy::{ExposeSecret, SecretString};
use subprocess::{Exec, ExitStatus, Redirection};
use thiserror::Error;
use tracing::{debug, info, warn};
use which::which;

use crate::args::NixBuildPassthroughArgs;

/// Privilege-elevation configuration captured from environment variables.
///
/// Replaces ad-hoc `env::var("NH_SUDOOPTS")` etc. reads. Construct once
/// in `main()` via `from_env()`, pass by reference.
#[derive(Debug, Clone)]
pub struct SudoConfig {
    /// `NH_SUDOOPTS` (preferred) or `NIX_SUDOOPTS` (legacy), shell-split.
    pub opts: Vec<String>,
    /// `NH_SUDO_ASKPASS` — path to askpass helper.
    pub askpass: Option<String>,
    /// `NH_PRESERVE_ENV` — defaults to `true` when unset; `false` when "0".
    pub preserve_env: bool,
}

impl Default for SudoConfig {
    fn default() -> Self {
        Self {
            opts: Vec::new(),
            askpass: None,
            preserve_env: true,
        }
    }
}

impl SudoConfig {
    /// Capture sudo-related env vars from the current process.
    ///
    /// Called once in `main()`. Tests should construct `SudoConfig` directly.
    #[must_use]
    pub fn from_env() -> Self {
        let opts = env::var("NH_SUDOOPTS")
            .or_else(|_| env::var("NIX_SUDOOPTS"))
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| {
                shlex::split(&s).unwrap_or_else(|| {
                    warn!(
                        "Failed to parse NH_SUDOOPTS/NIX_SUDOOPTS, \
                         ignoring. Value: {s}"
                    );
                    Vec::new()
                })
            })
            .unwrap_or_default();

        Self {
            opts,
            askpass: env::var("NH_SUDO_ASKPASS").ok(),
            preserve_env: env::var("NH_PRESERVE_ENV")
                .as_deref()
                .map_or(true, |x| !matches!(x, "0")),
        }
    }
}

#[must_use]
pub fn get_sudo_opts(config: &SudoConfig) -> Vec<String> {
    config.opts.clone()
}

/// Execute a command, streaming output to stdout/stderr while optionally
/// capturing it for error reporting.
///
/// # Arguments
///
/// * `capture_output` - When `true`, stdout and stderr are accumulated and
///   returned as strings. When `false`, output is streamed but not captured.
///
/// # Returns
///
/// Returns the exit status and captured stdout/stderr (or empty strings if
/// `capture_output` is `false`), or an error.
///
/// # Errors
///
/// Returns an error if:
///
/// - The command fails to start
/// - stdout or stderr cannot be captured
/// - The command fails to complete
/// - Either output thread panics
pub fn exec_with_streaming(
    cmd: Exec,
    capture_output: bool,
) -> Result<(subprocess::ExitStatus, String, String)> {
    let mut job = cmd
        .stdout(Redirection::Pipe)
        .start()
        .wrap_err("Failed to start command")?;

    let stdout_pipe = job
        .stdout
        .take()
        .ok_or_else(|| eyre::eyre!("Failed to capture stdout"))?;

    let stdout_thread = std::thread::spawn(move || {
        let mut stdout_reader = std::io::BufReader::new(stdout_pipe);
        let mut stdout_bytes = Vec::new();

        loop {
            let buf = match stdout_reader.fill_buf() {
                Ok(buf) => buf,
                Err(e) => {
                    debug!("stdout read error: {e}");
                    break;
                }
            };
            if buf.is_empty() {
                break;
            }
            let _ = std::io::stdout().write_all(buf);
            let _ = std::io::stdout().flush();
            if capture_output {
                stdout_bytes.extend_from_slice(buf);
            }
            let len = buf.len();
            stdout_reader.consume(len);
        }

        if capture_output {
            String::from_utf8_lossy(&stdout_bytes).into_owned()
        } else {
            String::new()
        }
    });

    let stderr_thread = job.stderr.take().map(|stderr_pipe| {
        std::thread::spawn(move || {
            let mut stderr_reader = std::io::BufReader::new(stderr_pipe);
            let mut stderr_bytes = Vec::new();

            loop {
                let buf = match stderr_reader.fill_buf() {
                    Ok(buf) => buf,
                    Err(e) => {
                        debug!("stderr read error: {e}");
                        break;
                    }
                };
                if buf.is_empty() {
                    break;
                }
                let _ = std::io::stderr().write_all(buf);
                let _ = std::io::stderr().flush();
                if capture_output {
                    stderr_bytes.extend_from_slice(buf);
                }
                let len = buf.len();
                stderr_reader.consume(len);
            }

            if capture_output {
                String::from_utf8_lossy(&stderr_bytes).into_owned()
            } else {
                String::new()
            }
        })
    });

    let exit_status = job
        .wait()
        .wrap_err("Failed to wait for command completion")?;

    let stdout_output = stdout_thread
        .join()
        .map_err(|_| eyre::eyre!("Stdout thread panicked"))?;
    let stderr_output = stderr_thread
        .map(|t| {
            t.join().map_err(|_| eyre::eyre!("Stderr thread panicked"))
        })
        .transpose()?
        .unwrap_or_default();

    Ok((exit_status, stdout_output, stderr_output))
}

static PASSWORD_CACHE: OnceLock<Mutex<HashMap<String, SecretString>>> =
    OnceLock::new();

/// Retrieves a cached password for the specified host.
///
/// # Arguments
///
/// * `host` - The host identifier (e.g., "user@hostname" or "hostname") to look
///   up in the cache
///
/// # Returns
///
/// * `Some(SecretString)` - If a password for the host exists in the cache
/// * `None` - If no password has been cached for this host
///
/// # Errors
///
/// Returns an error if the password cache lock is poisoned.
pub fn get_cached_password(host: &str) -> Result<Option<SecretString>> {
    let cache = PASSWORD_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let guard = cache
        .lock()
        .map_err(|_| eyre::eyre!("Password cache lock poisoned"))?;
    Ok(guard.get(host).cloned())
}

/// Stores a password in the cache for the specified host.
///
/// The password is stored as a `SecretString` to ensure secure memory
/// handling. Cached passwords persist for the lifetime of the program and can
/// be retrieved using [`get_cached_password`].
///
/// # Arguments
///
/// * `host` - The host identifier (e.g., "user@hostname" or "hostname") to
///   associate with the password
/// * `password` - The password to cache, wrapped in a `SecretString` for secure
///   handling
///
/// # Errors
///
/// Returns an error if the password cache lock is poisoned.
pub fn cache_password(host: &str, password: SecretString) -> Result<()> {
    let cache = PASSWORD_CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    cache
        .lock()
        .map_err(|_| eyre::eyre!("Password cache lock poisoned"))?
        .insert(host.to_string(), password);

    Ok(())
}

fn ssh_wrap(
    cmd: Exec,
    ssh: Option<&str>,
    password: Option<&SecretString>,
) -> Exec {
    if let Some(ssh) = ssh {
        let mut ssh_cmd = Exec::cmd("ssh")
            .arg("-T")
            .arg(ssh)
            .arg(cmd.to_cmdline_lossy());

        if let Some(pwd) = password {
            let stdin_data: Vec<u8> =
                format!("{}\n", pwd.expose_secret()).into_bytes();
            ssh_cmd = ssh_cmd.stdin(stdin_data);
        }

        ssh_cmd
    } else {
        cmd
    }
}

#[allow(dead_code)] // shut up
#[derive(Debug, Clone)]
pub enum EnvAction {
    /// Set an environment variable to a specific value
    Set(String),

    /// Preserve an environment variable from the current environment
    Preserve,

    /// Remove/unset an environment variable
    Remove,
}

/// Strategy argument for handling privilege elevation when running commands.
///
/// Defines how `nh` should handle privilege elevation for commands
/// that require root access (e.g., `switch-to-configuration`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevationStrategyArg {
    /// No elevation - commands run without privilege escalation.
    None,

    /// Automatically detect and use the first available elevation program
    /// (tries doas -> sudo -> run0 -> pkexec in order). Uses askpass helper if
    /// available.
    Auto,

    /// Use elevation program but skip password prompting for remote hosts with
    /// NOPASSWD configured.
    Passwordless,

    /// Use the specified elevation program.
    Program(PathBuf),
}

impl FromStr for ElevationStrategyArg {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Self::None),
            "auto" => Ok(Self::Auto),
            "passwordless" => Ok(Self::Passwordless),
            _ => s.strip_prefix("program:").map_or_else(
                || Ok(Self::Program(PathBuf::from(s))),
                |rest| Ok(Self::Program(PathBuf::from(rest))),
            ),
        }
    }
}

/// Strategy for handling privilege elevation at runtime.
///
/// This enum defines how `nh` should handle privilege elevation for commands
/// that require root access (e.g., `switch-to-configuration`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevationStrategy {
    /// Automatically detect and use the first available elevation program
    /// (tries doas -> sudo -> run0 -> pkexec in order). Uses askpass helper if
    /// available.
    Auto,

    /// Try the specified elevation program first, fall back to `Auto` if not
    /// found. Corresponds to CLI argument that is a path.
    Prefer(PathBuf),

    /// Use only the specified program name.
    #[allow(dead_code, reason = "In use")]
    Force(&'static str),

    /// Do not use any elevation program. Commands run without privilege
    /// escalation. This will fail for commands requiring root unless the user is
    /// already root or the system has other privilege mechanisms configured.
    None,

    /// Use elevation program but skip password prompting. For remote hosts with
    /// passwordless sudo (NOPASSWD in sudoers) or similar configurations. The
    /// elevation command runs without `--stdin` or password input.
    Passwordless,
}

impl ElevationStrategy {
    /// Resolves the elevation strategy to an actual program path.
    ///
    /// Attempts to find an appropriate privilege elevation program based on the
    /// strategy variant and system availability.
    ///
    /// # Returns
    ///
    /// Returns `Ok(PathBuf)` containing the path to the elevation program binary.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// - `None` variant: Always fails (elevation is disabled via
    ///   `--elevation-strategy=none`)
    /// - `Force` variant: The specified program is not found in PATH
    /// - Other variants: No suitable elevation programs are available on the
    ///   system
    pub fn resolve(&self) -> Result<PathBuf> {
        match self {
      Self::Auto | Self::Passwordless => Self::choice(),
      Self::Prefer(program) => {
        which(program).or_else(|_| {
          warn!(
            ?program,
            "Preferred elevation program not found, falling back to \
             auto-detection"
          );
          Self::choice()
        })
      },
      Self::Force(program_name) => {
        which(program_name).context(format!(
          "Forced elevation program '{program_name}' not found in PATH"
        ))
      },
      // Only reachable if resolve() is called directly. Safe since callers
      // check is_some() before invoking resolve().
      Self::None => bail!("Elevation disabled via --elevation-strategy=none"),
    }
    }

    /// Gets a path to a privilege elevation program based on what is available in
    /// the system.
    ///
    /// This funtion checks for the existence of common privilege elevation
    /// program names in the `PATH` using the `which` crate and returns a Ok
    /// result with the `OsString` of the path to the binary. In the case none
    /// of the checked programs are found a Err result is returned.
    ///
    /// The search is done in this order:
    ///
    /// 1. `doas`
    /// 2. `sudo`
    /// 3. `run0`
    /// 4. `pkexec`
    ///
    /// The logic for choosing this order is that a person with `doas` installed
    /// is more likely to be using it as their main privilege elevation program.
    /// `run0` and `pkexec` are preinstalled in any `NixOS` system with polkit
    /// support installed, so they have been placed lower as it's easier to
    /// deactivate sudo than it is to remove `run0`/`pkexec`
    ///
    /// # Returns
    ///
    /// * `Result<PathBuf>` - The absolute path to the privilege elevation program
    ///   binary or an error if a program can't be found.
    fn choice() -> Result<PathBuf> {
        const STRATEGIES: [&str; 4] = ["doas", "sudo", "run0", "pkexec"];

        for strategy in STRATEGIES {
            if let Ok(path) = which(strategy) {
                debug!(?path, "{strategy} path found");
                return Ok(path);
            }
        }

        Err(eyre::eyre!(
            "No elevation strategy found. Checked: {}",
            STRATEGIES.join(", ")
        ))
    }
}

#[derive(Debug)]
#[allow(clippy::struct_field_names)]
pub struct Command {
    dry: bool,
    message: Option<String>,
    command: OsString,
    args: Vec<OsString>,
    elevate: Option<ElevationStrategy>,
    ssh: Option<String>,
    show_output: bool,
    env_vars: HashMap<String, EnvAction>,
    subprocess_env: SubprocessEnv,
    sudo_config: SudoConfig,
}

impl Command {
    pub fn new<S: AsRef<OsStr>>(
        command: S,
        subprocess_env: &SubprocessEnv,
        sudo_config: &SudoConfig,
    ) -> Self {
        Self {
            dry: false,
            message: None,
            command: command.as_ref().to_os_string(),
            args: vec![],
            elevate: None,
            ssh: None,
            show_output: false,
            env_vars: HashMap::new(),
            subprocess_env: subprocess_env.clone(),
            sudo_config: sudo_config.clone(),
        }
    }

    /// Set whether to run the command with elevated privileges.
    #[must_use]
    pub fn elevate(mut self, elevate: Option<ElevationStrategy>) -> Self {
        self.elevate = elevate;
        self
    }

    /// Set whether to perform a dry run.
    #[must_use]
    pub const fn dry(mut self, dry: bool) -> Self {
        self.dry = dry;
        self
    }

    /// Set whether to show command output.
    #[must_use]
    pub const fn show_output(mut self, show_output: bool) -> Self {
        self.show_output = show_output;
        self
    }

    /// Add a single argument to the command.
    #[must_use]
    pub fn arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    /// Add multiple arguments to the command.
    #[must_use]
    pub fn args<I>(mut self, args: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        for elem in args {
            self.args.push(elem.as_ref().to_os_string());
        }
        self
    }

    /// Set a message to display before running the command.
    #[must_use]
    pub fn message<S: AsRef<str>>(mut self, message: S) -> Self {
        self.message = Some(message.as_ref().to_string());
        self
    }

    /// Preserve multiple environment variables from the current environment
    #[must_use]
    pub fn preserve_envs<I, K>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = K>,
        K: AsRef<str>,
    {
        for key in keys {
            let key_str = key.as_ref().to_string();
            self.env_vars.insert(key_str, EnvAction::Preserve);
        }
        self
    }

    /// Set an environment variable to a specific value
    #[must_use]
    pub fn set_env<K, V>(mut self, key: K, value: V) -> Self
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.env_vars.insert(
            key.as_ref().to_string(),
            EnvAction::Set(value.as_ref().to_string()),
        );
        self
    }

    /// Apply subprocess environment from the stored `SubprocessEnv`.
    ///
    /// Replaces the old `with_required_env()` which read `env::var`
    /// directly. The env was captured at `Command::new` time.
    #[must_use]
    pub fn with_env(mut self) -> Self {
        if let Some(user) = &self.subprocess_env.user {
            self.env_vars
                .insert("USER".to_string(), EnvAction::Set(user.clone()));
        }

        // Only propagate HOME for non-elevated commands
        if self.elevate.is_none()
            && let Some(home) = &self.subprocess_env.home
        {
            self.env_vars
                .insert("HOME".to_string(), EnvAction::Set(home.clone()));
        }

        // Preserve all captured Nix-related variables
        for (key, _value) in &self.subprocess_env.nix_preserve {
            self.env_vars.insert(key.clone(), EnvAction::Preserve);
        }

        // Explicitly set NH_* variables
        for (key, value) in &self.subprocess_env.nh_vars {
            self.env_vars
                .insert(key.clone(), EnvAction::Set(value.clone()));
        }

        debug!(
            "Configured envs: {}",
            self.env_vars
                .iter()
                .map(|(key, action)| {
                    match action {
                        EnvAction::Set(value) => format!("{key}={value}"),
                        EnvAction::Preserve => {
                            format!("{key}=<preserved>")
                        }
                        EnvAction::Remove => format!("{key}=<removed>"),
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        );

        self
    }

    fn apply_env_to_exec(&self, mut cmd: Exec) -> Exec {
        for (key, action) in &self.env_vars {
            match action {
                EnvAction::Set(value) => {
                    cmd = cmd.env(key, value);
                }
                EnvAction::Preserve => {
                    // Resolve from captured subprocess env
                    if let Some(value) = self.subprocess_env.lookup(key) {
                        cmd = cmd.env(key, value);
                    }
                }
                EnvAction::Remove => {
                    // For remove, we'll handle this in the sudo construction
                    // by not including it in preserved variables
                }
            }
        }
        cmd
    }

    /// Creates a Exec that contains elevates the program with proper environment
    /// handling.
    ///
    /// Panics: If called when `self.elevate` is `None`
    fn build_sudo_cmd(&self) -> Result<Exec> {
        let elevation_strategy =
            self.elevate.as_ref().ok_or_else(|| {
                eyre::eyre!("Command not found for elevation")
            })?;

        let elevation_program = elevation_strategy
            .resolve()
            .context("Failed to resolve elevation program")?;

        let mut cmd = Exec::cmd(&elevation_program);

        // Use NH_SUDO_ASKPASS program for sudo if present, but NOT for
        // Passwordless variant (Passwordless expects NOPASSWD config without
        // password input)
        let program_name = elevation_program
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                eyre::eyre!("Failed to determine elevation program name")
            })?;
        if program_name == "sudo"
            && !matches!(
                elevation_strategy,
                ElevationStrategy::Passwordless
            )
            && let Some(askpass) = &self.sudo_config.askpass
        {
            cmd = cmd.env("SUDO_ASKPASS", askpass).arg("-A");
        }
        // Request allocation of a pseudo TTY for the run0 session. Without this,
        // running `run0` changes the user of `/dev/pts/<current-terminal>
        // to `root`, which we want to avoid since it can cause issues with
        // subsequent commands.
        if program_name == "run0" {
            cmd = cmd.arg("--pty-late");
        }

        if program_name == "sudo" {
            cmd = cmd.args(get_sudo_opts(&self.sudo_config));
        }

        let preserve_env = self.sudo_config.preserve_env;

        // Insert 'env' command to explicitly pass environment variables to the
        // elevated command
        cmd = cmd.arg("env");
        for arg in
            self.env_vars
                .iter()
                .filter_map(|(key, action)| match action {
                    EnvAction::Set(value) => {
                        Some(format!("{key}={value}"))
                    }
                    EnvAction::Preserve if preserve_env => self
                        .subprocess_env
                        .lookup(key)
                        .map(|value| format!("{key}={value}")),
                    _ => None,
                })
        {
            cmd = cmd.arg(arg);
        }

        Ok(cmd)
    }

    fn build_sudo_parts(&self) -> Result<Vec<String>> {
        let elevation_program = self
            .elevate
            .as_ref()
            .ok_or_else(|| eyre::eyre!("Command not found for elevation"))?
            .resolve()
            .context("Failed to resolve elevation program")?;

        let mut parts =
            vec![elevation_program.to_string_lossy().to_string()];

        let program_name = elevation_program
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                eyre::eyre!("Failed to determine elevation program name")
            })?;
        if program_name == "sudo"
            && self.sudo_config.askpass.is_some()
        {
            parts.push("-A".to_string());
        }
        // Request allocation of a pseudo TTY for the run0 session. Without this,
        // running `run0` changes the user of `/dev/pts/<current-terminal>
        // to `root`, which we want to avoid since it can cause issues with
        // subsequent commands.
        if program_name == "run0" {
            parts.push("--pty-late".to_string());
        }

        if program_name == "sudo" {
            parts.extend(get_sudo_opts(&self.sudo_config));
        }

        let preserve_env = self.sudo_config.preserve_env;

        parts.push("env".to_string());
        for env_arg in
            self.env_vars
                .iter()
                .filter_map(|(key, action)| match action {
                    EnvAction::Set(value) => {
                        Some(format!("{key}={value}"))
                    }
                    EnvAction::Preserve if preserve_env => self
                        .subprocess_env
                        .lookup(key)
                        .map(|value| format!("{key}={value}")),
                    _ => None,
                })
        {
            parts.push(env_arg);
        }

        Ok(parts)
    }

    /// Create a sudo command for self-elevation with proper environment handling
    ///
    /// # Errors
    ///
    /// Returns an error if the current executable path cannot be determined or
    /// sudo command cannot be built.
    pub fn self_elevate_cmd(
        strategy: ElevationStrategy,
        subprocess_env: &SubprocessEnv,
        sudo_config: &SudoConfig,
    ) -> Result<std::process::Command> {
        // Get the current executable path
        let current_exe = env::current_exe()
            .context("Failed to get current executable path")?;

        // Self-elevation with proper environment handling
        let cmd_builder = Self::new(&current_exe, subprocess_env, sudo_config)
            .elevate(Some(strategy))
            .with_env();

        let mut sudo_parts = cmd_builder.build_sudo_parts()?;

        // The first element is always the elevation program.
        let uses_askpass = sudo_parts.iter().any(|p| p == "-A");

        // Add the target executable and arguments
        sudo_parts.push(current_exe.to_string_lossy().to_string());
        let args: Vec<String> = env::args().skip(1).collect();
        sudo_parts.extend(args);

        let mut parts_iter = sudo_parts.into_iter();
        let program = parts_iter
            .next()
            .ok_or_else(|| eyre::eyre!("Elevation program is missing"))?;
        let mut std_cmd = std::process::Command::new(program);
        std_cmd.args(parts_iter);

        // check if using SUDO_ASKPASS
        if uses_askpass
            && let Some(askpass) = &sudo_config.askpass
        {
            std_cmd.env("SUDO_ASKPASS", askpass);
        }
        Ok(std_cmd)
    }

    /// Run the configured command.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to execute or returns a non-zero
    /// exit status.
    ///
    /// # Panics
    ///
    /// Panics if the command result is unexpectedly None.
    pub fn run(&self) -> Result<()> {
        // Prompt for elevation password if needed for remote deployment.
        // Note: Only sudo supports stdin password input. For remote deployments
        // with doas/run0, use --elevation-strategy=passwordless instead.
        let sudo_password = if self.ssh.is_some() && self.elevate.is_some()
        {
            let host = self.ssh.as_ref().ok_or_else(|| {
                eyre::eyre!("SSH host is None but elevation is required")
            })?;
            if let Some(cached_password) = get_cached_password(host)? {
                Some(cached_password)
            } else {
                let password = inquire::Password::new(&format!(
                    "[sudo] password for {host}:"
                ))
                .without_confirmation()
                .prompt()
                .context("Failed to read sudo password")?;
                if password.is_empty() {
                    bail!("Password cannot be empty");
                }
                let secret_password = SecretString::new(password.into());
                cache_password(host, secret_password.clone())?;
                Some(secret_password)
            }
        } else {
            None
        };

        let cmd = if self.elevate.is_some() && self.ssh.is_none() {
            // Local elevation
            self.build_sudo_cmd()?.arg(&self.command).args(&self.args)
        } else if self.elevate.is_some() && self.ssh.is_some() {
            // Build elevation command
            let elevation_program = self
        .elevate
        .as_ref()
        .ok_or_else(|| {
          eyre::eyre!("Elevation program is None but elevation is required")
        })?
        .resolve()
        .context("Failed to resolve elevation program")?;

            let program_name = elevation_program
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    eyre::eyre!(
                        "Failed to determine elevation program name"
                    )
                })?;

            let mut elev_cmd = Exec::cmd(&elevation_program);

            // Add program-specific arguments
            if program_name == "sudo" {
                elev_cmd = elev_cmd.arg("--prompt=").arg("--stdin");
                elev_cmd = elev_cmd.args(get_sudo_opts(&self.sudo_config));
            }

            // Add env command to handle environment variables
            elev_cmd = elev_cmd.arg("env");
            for (key, action) in &self.env_vars {
                match action {
                    EnvAction::Set(value) => {
                        let quoted_value = shlex::try_quote(value)
                            .unwrap_or_else(|_| value.clone().into());
                        elev_cmd =
                            elev_cmd.arg(format!("{key}={quoted_value}"));
                    }
                    EnvAction::Preserve => {
                        if let Some(value) = self.subprocess_env.lookup(key) {
                            let quoted_value = shlex::try_quote(value)
                                .unwrap_or_else(|_| value.to_string().into());
                            elev_cmd = elev_cmd
                                .arg(format!("{key}={quoted_value}"));
                        }
                    }
                    EnvAction::Remove => {}
                }
            }

            elev_cmd.arg(&self.command).args(&self.args)
        } else {
            // No elevation
            self.apply_env_to_exec(
                Exec::cmd(&self.command).args(&self.args),
            )
        };

        // Configure output redirection based on show_output setting
        let cmd = ssh_wrap(
            if self.show_output {
                cmd.stderr(Redirection::Merge)
            } else {
                cmd.stderr(Redirection::None).stdout(Redirection::None)
            },
            self.ssh.as_deref(),
            sudo_password.as_ref(),
        );

        if let Some(m) = &self.message {
            info!("{m}");
        }

        debug!(?cmd);

        if self.dry {
            return Ok(());
        }

        let msg = self
            .message
            .clone()
            .unwrap_or_else(|| "Command failed".to_string());

        if self.show_output {
            let exit_status = cmd.join().wrap_err(msg.clone())?;
            if !exit_status.success() {
                return Err(eyre::eyre!(format!(
                    "{} (exit status {:?})",
                    msg, exit_status
                )));
            }
            Ok(())
        } else {
            let res = cmd.capture();
            match res {
                Ok(capture) => {
                    let status = &capture.exit_status;
                    if !status.success() {
                        let stderr = capture.stderr_str();
                        if stderr.trim().is_empty() {
                            return Err(eyre::eyre!(format!(
                                "{} (exit status {:?})",
                                msg, status
                            )));
                        }
                        return Err(eyre::eyre!(format!(
                            "{} (exit status {:?})\nstderr:\n{}",
                            msg, status, stderr
                        )));
                    }
                    Ok(())
                }
                Err(e) => Err(e).wrap_err(msg),
            }
        }
    }

    /// Run the configured command and capture its output.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to execute.
    pub fn run_capture(&self) -> Result<Option<String>> {
        let cmd = self.apply_env_to_exec(
            Exec::cmd(&self.command)
                .args(&self.args)
                .stderr(Redirection::None)
                .stdout(Redirection::Pipe),
        );

        if let Some(m) = &self.message {
            info!("{m}");
        }

        debug!(?cmd);

        if self.dry {
            return Ok(None);
        }
        Ok(Some(cmd.capture()?.stdout_str()))
    }
}

#[derive(Debug)]
pub struct Build {
    message: Option<String>,
    installable: Installable,
    extra_args: Vec<OsString>,
    nom: bool,
}

impl Build {
    #[must_use]
    pub const fn new(installable: Installable) -> Self {
        Self {
            message: None,
            installable,
            extra_args: vec![],
            nom: false,
        }
    }

    #[must_use]
    pub fn message<S: AsRef<str>>(mut self, message: S) -> Self {
        self.message = Some(message.as_ref().to_string());
        self
    }

    #[must_use]
    pub fn extra_arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
        self.extra_args.push(arg.as_ref().to_os_string());
        self
    }

    #[must_use]
    pub const fn nom(mut self, yes: bool) -> Self {
        self.nom = yes;
        self
    }

    #[must_use]
    pub fn extra_args<I>(mut self, args: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        for elem in args {
            self.extra_args.push(elem.as_ref().to_os_string());
        }
        self
    }

    #[must_use]
    pub fn passthrough(
        self,
        passthrough: &NixBuildPassthroughArgs,
    ) -> Self {
        self.extra_args(passthrough.generate_passthrough_args())
    }

    /// Run the build command.
    ///
    /// # Errors
    ///
    /// Returns an error if the build command fails to execute.
    pub fn run(&self) -> Result<()> {
        if let Some(m) = &self.message {
            info!("{m}");
        }

        let installable_args = self.installable.to_args();

        let base_command = NixCommand::new(CommandKind::Build)
            .print_build_logs(false)
            .args(&installable_args)
            .args(&self.extra_args)
            .to_exec();

        if self.nom {
            let pipeline = {
                base_command
                    .args(["--log-format", "internal-json", "--verbose"])
                    .stderr(Redirection::Merge)
                    .stdout(Redirection::Pipe)
                    | Exec::cmd("nom").args(["--json"])
            }
            .stdout(Redirection::None);
            debug!(?pipeline);

            // Use `popen()` to get access to individual processes so we can check
            // Nix's exit status, not nom's. The pipeline's `join()` only returns
            // the exit status of the last command (nom), which always succeeds
            // even when Nix fails.
            let job = pipeline.start()?;

            // Wait for all processes to finish
            for proc in &job.processes {
                proc.wait()?;
            }

            // Check the exit status of the FIRST process (nix build)
            // This is the one that matters. If Nix fails, we should fail as well
            if let Some(nix_proc) = job.processes.first() {
                let exit_status = nix_proc.wait()?;
                if !exit_status.success() {
                    bail!(ExitError(exit_status));
                }
            }
        } else {
            let cmd = base_command
                .stderr(Redirection::Merge)
                .stdout(Redirection::None);

            debug!(?cmd);
            let exit = cmd.join();

            let exit_status = exit?;
            if !exit_status.success() {
                bail!(ExitError(exit_status));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Error)]
#[error("Command exited with status {0:?}")]
pub struct ExitError(ExitStatus);

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::unreachable,
        reason = "Fine in tests"
    )]
    use std::ffi::OsString;

    use super::*;

    // ---- helpers ----

    fn env_default() -> SubprocessEnv {
        SubprocessEnv::default()
    }

    fn sudo_default() -> SudoConfig {
        SudoConfig::default()
    }

    fn env_with(
        home: Option<&str>,
        user: Option<&str>,
        nix_preserve: Vec<(&str, &str)>,
        nh_vars: Vec<(&str, &str)>,
    ) -> SubprocessEnv {
        SubprocessEnv::from_pairs(user, home, nix_preserve, nh_vars)
    }

    fn sudo_with(opts: Vec<&str>, askpass: Option<&str>, preserve_env: bool) -> SudoConfig {
        SudoConfig {
            opts: opts.into_iter().map(String::from).collect(),
            askpass: askpass.map(String::from),
            preserve_env,
        }
    }

    // ---- EnvAction ----

    #[test]
    fn test_env_action_variants() {
        let set_action = EnvAction::Set("test_value".to_string());
        let preserve_action = EnvAction::Preserve;
        let remove_action = EnvAction::Remove;

        match set_action {
            EnvAction::Set(val) => assert_eq!(val, "test_value"),
            _ => unreachable!("Expected Set variant"),
        }

        assert!(matches!(preserve_action, EnvAction::Preserve));
        assert!(matches!(remove_action, EnvAction::Remove));
    }

    #[test]
    fn test_env_action_debug() {
        let set_action = EnvAction::Set("value".to_string());
        let preserve_action = EnvAction::Preserve;
        let remove_action = EnvAction::Remove;

        let _debug_set = format!("{set_action:?}");
        let _debug_preserve = format!("{preserve_action:?}");
        let _debug_remove = format!("{remove_action:?}");
    }

    #[test]
    fn test_env_action_clone() {
        let original = EnvAction::Set("value".to_string());
        let cloned = original.clone();

        match (original, cloned) {
            (EnvAction::Set(orig_val), EnvAction::Set(cloned_val)) => {
                assert_eq!(orig_val, cloned_val);
            }
            #[allow(clippy::unreachable, reason = "Should never happen")]
            _ => unreachable!("Clone should preserve variant and value"),
        }
    }

    // ---- Command::new ----

    #[test]
    fn test_command_new() {
        let cmd = Command::new("test-command", &env_default(), &sudo_default());

        assert_eq!(cmd.command, OsString::from("test-command"));
        assert!(!cmd.dry);
        assert!(cmd.message.is_none());
        assert!(cmd.args.is_empty());
        assert!(cmd.elevate.is_none());
        assert!(cmd.ssh.is_none());
        assert!(!cmd.show_output);
        assert!(cmd.env_vars.is_empty());
    }

    #[test]
    fn test_command_builder_pattern() {
        let cmd = Command::new("test", &env_default(), &sudo_default())
            .dry(true)
            .show_output(true)
            .elevate(Some(ElevationStrategy::Force("sudo")))
            .message("test message")
            .arg("arg1")
            .args(["arg2", "arg3"]);

        assert!(cmd.dry);
        assert!(cmd.show_output);
        assert_eq!(cmd.elevate, Some(ElevationStrategy::Force("sudo")));
        assert_eq!(cmd.message, Some("test message".to_string()));
        assert_eq!(
            cmd.args,
            vec![
                OsString::from("arg1"),
                OsString::from("arg2"),
                OsString::from("arg3")
            ]
        );
    }

    #[test]
    fn test_preserve_envs() {
        let cmd = Command::new("test", &env_default(), &sudo_default())
            .preserve_envs(["VAR1", "VAR2", "VAR3"]);

        assert_eq!(cmd.env_vars.len(), 3);
        assert!(matches!(
            cmd.env_vars.get("VAR1"),
            Some(EnvAction::Preserve)
        ));
        assert!(matches!(
            cmd.env_vars.get("VAR2"),
            Some(EnvAction::Preserve)
        ));
        assert!(matches!(
            cmd.env_vars.get("VAR3"),
            Some(EnvAction::Preserve)
        ));
    }

    // ---- with_env ----

    #[test]
    fn test_with_env_home_user() {
        let env = env_with(
            Some("/test/home"),
            Some("testuser"),
            vec![],
            vec![],
        );
        let cmd = Command::new("test", &env, &sudo_default()).with_env();

        assert!(
            matches!(cmd.env_vars.get("HOME"), Some(EnvAction::Set(val)) if val == "/test/home")
        );
        assert!(
            matches!(cmd.env_vars.get("USER"), Some(EnvAction::Set(val)) if val == "testuser")
        );
    }

    #[test]
    fn test_with_env_missing_home_user() {
        let env = env_default();
        let cmd = Command::new("test", &env, &sudo_default()).with_env();

        assert!(!cmd.env_vars.contains_key("HOME"));
        assert!(!cmd.env_vars.contains_key("USER"));
    }

    #[test]
    fn test_with_env_nh_vars() {
        let env = env_with(
            None,
            None,
            vec![],
            vec![
                ("NH_TEST_VAR", "test_value"),
                ("NH_ANOTHER_VAR", "another_value"),
            ],
        );
        let cmd = Command::new("test", &env, &sudo_default()).with_env();

        assert!(
            matches!(cmd.env_vars.get("NH_TEST_VAR"), Some(EnvAction::Set(val)) if val == "test_value")
        );
        assert!(
            matches!(cmd.env_vars.get("NH_ANOTHER_VAR"), Some(EnvAction::Set(val)) if val == "another_value")
        );
    }

    #[test]
    fn test_with_env_nix_preserve() {
        let env = env_with(
            None,
            None,
            vec![
                ("PATH", "/usr/bin"),
                ("NIX_CONFIG", "experimental-features = flakes"),
            ],
            vec![],
        );
        let cmd = Command::new("test", &env, &sudo_default()).with_env();

        assert!(matches!(
            cmd.env_vars.get("PATH"),
            Some(EnvAction::Preserve)
        ));
        assert!(matches!(
            cmd.env_vars.get("NIX_CONFIG"),
            Some(EnvAction::Preserve)
        ));
    }

    #[test]
    fn test_with_env_combined() {
        let env = env_with(
            Some("/test/home"),
            None,
            vec![("PATH", "/usr/bin")],
            vec![("NH_TEST", "nh_value")],
        );
        let cmd = Command::new("test", &env, &sudo_default())
            .with_env()
            .preserve_envs(["EXTRA_VAR"]);

        assert!(
            matches!(cmd.env_vars.get("HOME"), Some(EnvAction::Set(val)) if val == "/test/home")
        );
        assert!(
            matches!(cmd.env_vars.get("NH_TEST"), Some(EnvAction::Set(val)) if val == "nh_value")
        );
        assert!(matches!(
            cmd.env_vars.get("PATH"),
            Some(EnvAction::Preserve)
        ));
        assert!(matches!(
            cmd.env_vars.get("EXTRA_VAR"),
            Some(EnvAction::Preserve)
        ));
    }

    #[test]
    fn test_env_vars_override_behavior() {
        let mut cmd = Command::new("test", &env_default(), &sudo_default());

        cmd.env_vars
            .insert("TEST_VAR".to_string(), EnvAction::Preserve);
        assert!(matches!(
            cmd.env_vars.get("TEST_VAR"),
            Some(EnvAction::Preserve)
        ));

        cmd.env_vars.insert(
            "TEST_VAR".to_string(),
            EnvAction::Set("new_value".to_string()),
        );
        assert!(
            matches!(cmd.env_vars.get("TEST_VAR"), Some(EnvAction::Set(val)) if val == "new_value")
        );
    }

    // ---- build_sudo_cmd ----

    #[test]
    fn test_build_sudo_cmd_basic() {
        let cmd = Command::new("test", &env_default(), &sudo_default())
            .elevate(Some(ElevationStrategy::Force("sudo")));
        let sudo_exec = cmd
            .build_sudo_cmd()
            .expect("build_sudo_cmd should succeed in test");

        let cmdline = sudo_exec.to_cmdline_lossy();
        assert!(
            cmdline.split_whitespace().any(|tok| tok.ends_with("sudo"))
        );
    }

    #[test]
    fn test_build_sudo_cmd_force_no_stdin() {
        let cmd = Command::new("test", &env_default(), &sudo_default())
            .elevate(Some(ElevationStrategy::Force("sudo")));

        let sudo_exec =
            cmd.build_sudo_cmd().expect("build_sudo_cmd should succeed");
        let cmdline = sudo_exec.to_cmdline_lossy();

        assert!(cmdline.contains("sudo"));
    }

    #[test]
    fn test_build_sudo_cmd_with_preserve_vars() {
        let env = env_with(
            None,
            None,
            vec![("VAR1", "1"), ("VAR2", "2")],
            vec![],
        );
        let sudo = sudo_with(vec![], None, true);

        let cmd = Command::new("test", &env, &sudo)
            .preserve_envs(["VAR1", "VAR2"])
            .elevate(Some(ElevationStrategy::Force("sudo")));

        let sudo_exec = cmd
            .build_sudo_cmd()
            .expect("build_sudo_cmd should succeed in test");
        let cmdline = sudo_exec.to_cmdline_lossy();

        assert!(cmdline.contains("env"));
        assert!(cmdline.contains("VAR1=1"));
        assert!(cmdline.contains("VAR2=2"));
    }

    #[test]
    fn test_build_sudo_cmd_with_preserve_vars_disabled() {
        let env = env_with(
            None,
            None,
            vec![("VAR1", "1"), ("VAR2", "2")],
            vec![],
        );
        let sudo = sudo_with(vec![], None, false);

        let cmd = Command::new("test", &env, &sudo)
            .preserve_envs(["VAR1", "VAR2"])
            .elevate(Some(ElevationStrategy::Force("sudo")));

        let sudo_exec = cmd
            .build_sudo_cmd()
            .expect("build_sudo_cmd should succeed in test");
        let cmdline = sudo_exec.to_cmdline_lossy();

        assert!(cmdline.contains("env"));
        assert!(!cmdline.contains("VAR1=1"));
        assert!(!cmdline.contains("VAR2=2"));
    }

    #[test]
    fn test_build_sudo_cmd_with_set_vars() {
        let mut cmd = Command::new("test", &env_default(), &sudo_default())
            .elevate(Some(ElevationStrategy::Force("sudo")));
        cmd.env_vars.insert(
            "TEST_VAR".to_string(),
            EnvAction::Set("test_value".to_string()),
        );

        let sudo_exec = cmd
            .build_sudo_cmd()
            .expect("build_sudo_cmd should succeed in test");
        let cmdline = sudo_exec.to_cmdline_lossy();

        assert!(cmdline.contains("env"));
        assert!(cmdline.contains("TEST_VAR=test_value"));
    }

    #[test]
    fn test_build_sudo_cmd_with_remove_vars() {
        let env = env_with(
            None,
            None,
            vec![("VAR_TO_PRESERVE", "preserve")],
            vec![],
        );

        let mut cmd = Command::new("test", &env, &sudo_default())
            .elevate(Some(ElevationStrategy::Force("sudo")));
        cmd.env_vars
            .insert("VAR_TO_PRESERVE".to_string(), EnvAction::Preserve);
        cmd.env_vars
            .insert("VAR_TO_REMOVE".to_string(), EnvAction::Remove);

        let sudo_exec = cmd
            .build_sudo_cmd()
            .expect("build_sudo_cmd should succeed in test");
        let cmdline = sudo_exec.to_cmdline_lossy();

        assert!(cmdline.contains("env"));
        assert!(cmdline.contains("VAR_TO_PRESERVE=preserve"));
        assert!(!cmdline.contains("VAR_TO_REMOVE"));
    }

    #[test]
    fn test_build_sudo_cmd_with_askpass() {
        let sudo = sudo_with(vec![], Some("/path/to/askpass"), true);

        let cmd = Command::new("test", &env_default(), &sudo)
            .elevate(Some(ElevationStrategy::Force("sudo")));
        let sudo_exec = cmd
            .build_sudo_cmd()
            .expect("build_sudo_cmd should succeed in test");
        let cmdline = sudo_exec.to_cmdline_lossy();

        assert!(cmdline.contains("-A"));
    }

    #[test]
    fn test_build_sudo_cmd_env_added_once() {
        let env = env_with(
            None,
            None,
            vec![("PRESERVE_VAR", "preserve")],
            vec![],
        );

        let mut cmd = Command::new("test", &env, &sudo_default())
            .elevate(Some(ElevationStrategy::Force("sudo")));
        cmd.env_vars.insert(
            "TEST_VAR1".to_string(),
            EnvAction::Set("value1".to_string()),
        );
        cmd.env_vars.insert(
            "TEST_VAR2".to_string(),
            EnvAction::Set("value2".to_string()),
        );
        cmd.env_vars
            .insert("PRESERVE_VAR".to_string(), EnvAction::Preserve);

        let sudo_exec = cmd
            .build_sudo_cmd()
            .expect("build_sudo_cmd should succeed in test");
        let cmdline = sudo_exec.to_cmdline_lossy();

        let env_count = cmdline.matches(" env ").count()
            + usize::from(cmdline.starts_with("env "))
            + usize::from(cmdline.ends_with(" env"));

        assert_eq!(
            env_count, 1,
            "env command should appear exactly once in: {cmdline}"
        );

        assert!(cmdline.contains("TEST_VAR1=value1"));
        assert!(cmdline.contains("TEST_VAR2=value2"));
        assert!(cmdline.contains("PRESERVE_VAR=preserve"));
    }

    #[test]
    fn test_build_sudo_cmd_with_nix_config_spaces() {
        let env = env_with(
            None,
            None,
            vec![(
                "NIX_CONFIG",
                "access-tokens = github.com=ghp_11111aaaaa22222bbbbb",
            )],
            vec![],
        );

        let cmd = Command::new("test", &env, &sudo_default())
            .elevate(Some(ElevationStrategy::Force("sudo")))
            .with_env();

        let sudo_exec = cmd
            .build_sudo_cmd()
            .expect("build_sudo_cmd should succeed in test");
        let cmdline = sudo_exec.to_cmdline_lossy();

        assert!(cmdline.contains(
      "'NIX_CONFIG=access-tokens = github.com=ghp_11111aaaaa22222bbbbb'"
    ));
        assert!(cmdline.contains("'NIX_CONFIG="));
        assert!(cmdline.contains("'NIX_CONFIG="));
    }

    // ---- ElevationStrategy ----

    #[test]
    fn test_elevation_strategy_passwordless_resolves() {
        let strategy = ElevationStrategy::Passwordless;
        let result = strategy.resolve();

        assert!(result.is_ok());
        let program = result.unwrap();
        assert!(!program.as_os_str().is_empty());
    }

    #[test]
    fn test_elevation_strategy_arg_program_prefix_parsing() {
        let parsed =
            "program:/path/to/bin".parse::<ElevationStrategyArg>();
        assert!(parsed.is_ok());
        match parsed.unwrap() {
            ElevationStrategyArg::Program(path) => {
                assert_eq!(path, PathBuf::from("/path/to/bin"));
            }
            _ => unreachable!("Expected Program variant"),
        }
    }

    // ---- Build struct ----

    #[test]
    fn test_build_new() {
        let installable = Installable::Flake {
            reference: "github:user/repo".to_string(),
            attribute: vec!["package".to_string()],
        };

        let build = Build::new(installable.clone());

        assert!(build.message.is_none());
        assert_eq!(build.installable.to_args(), installable.to_args());
        assert!(build.extra_args.is_empty());
        assert!(!build.nom);
    }

    #[test]
    fn test_build_builder_pattern() {
        let installable = Installable::Flake {
            reference: "github:user/repo".to_string(),
            attribute: vec!["package".to_string()],
        };

        let build = Build::new(installable)
            .message("Building package")
            .extra_arg("--verbose")
            .extra_args(["--option", "setting", "value"])
            .nom(true);

        assert_eq!(build.message, Some("Building package".to_string()));
        assert_eq!(
            build.extra_args,
            vec![
                OsString::from("--verbose"),
                OsString::from("--option"),
                OsString::from("setting"),
                OsString::from("value")
            ]
        );
        assert!(build.nom);
    }

    // ---- ssh_wrap ----

    #[test]
    fn test_ssh_wrap_with_ssh() {
        let cmd = subprocess::Exec::cmd("echo").arg("hello");
        let wrapped = ssh_wrap(cmd, Some("user@host"), None);

        let cmdline = wrapped.to_cmdline_lossy();
        assert!(cmdline.starts_with("ssh"));
        assert!(cmdline.contains("-T"));
        assert!(cmdline.contains("user@host"));
    }

    #[test]
    fn test_ssh_wrap_without_ssh() {
        let cmd = subprocess::Exec::cmd("echo").arg("hello");
        let expected = cmd.to_cmdline_lossy();
        let wrapped = ssh_wrap(cmd, None, None);

        assert_eq!(wrapped.to_cmdline_lossy(), expected);
    }

    #[test]
    fn test_ssh_wrap_with_password() {
        let cmd = subprocess::Exec::cmd("echo").arg("hello");
        let password = SecretString::new("testpass".into());
        let wrapped = ssh_wrap(cmd, Some("user@host"), Some(&password));

        let cmdline = wrapped.to_cmdline_lossy();
        assert!(cmdline.starts_with("ssh"));
        assert!(cmdline.contains("-T"));
        assert!(cmdline.contains("user@host"));
    }

    // ---- apply_env_to_exec ----

    #[test]
    fn test_apply_env_to_exec() {
        let env = env_with(
            None,
            None,
            vec![("EXISTING_VAR", "existing_value")],
            vec![],
        );

        let mut cmd = Command::new("test", &env, &sudo_default());
        cmd.env_vars.insert(
            "SET_VAR".to_string(),
            EnvAction::Set("set_value".to_string()),
        );
        cmd.env_vars
            .insert("EXISTING_VAR".to_string(), EnvAction::Preserve);
        cmd.env_vars
            .insert("MISSING_VAR".to_string(), EnvAction::Preserve);
        cmd.env_vars
            .insert("REMOVE_VAR".to_string(), EnvAction::Remove);

        let exec = subprocess::Exec::cmd("echo");
        let result = cmd.apply_env_to_exec(exec);

        let cmdline = result.to_cmdline_lossy();
        assert!(
            cmdline.contains("echo"),
            "Command line should contain 'echo': {cmdline}"
        );
    }

    // ---- ExitError ----

    #[test]
    fn test_exit_error_display() {
        let exit_status = subprocess::Exec::cmd("false")
            .join()
            .expect("failed to run 'false'");
        let error = ExitError(exit_status);

        let error_string = format!("{error}");
        assert!(error_string.contains("Command exited with status"));
    }

    // ---- shlex parsing ----

    #[test]
    fn test_parse_cmdline_simple() {
        let result =
            shlex::split("cmd arg1 arg2 arg3").unwrap_or_default();
        assert_eq!(result, vec!["cmd", "arg1", "arg2", "arg3"]);
    }

    #[test]
    fn test_parse_cmdline_with_single_quotes() {
        let result =
            shlex::split("cmd 'arg with spaces' arg2").unwrap_or_default();
        assert_eq!(result, vec!["cmd", "arg with spaces", "arg2"]);
    }

    #[test]
    fn test_parse_cmdline_with_double_quotes() {
        let result = shlex::split(r#"cmd "arg with spaces" arg2"#)
            .unwrap_or_default();
        assert_eq!(result, vec!["cmd", "arg with spaces", "arg2"]);
    }

    #[test]
    fn test_parse_cmdline_mixed_quotes() {
        let result =
            shlex::split(r#"cmd 'single quoted' "double quoted" normal"#)
                .unwrap_or_default();
        assert_eq!(
            result,
            vec!["cmd", "single quoted", "double quoted", "normal"]
        );
    }

    #[test]
    fn test_parse_cmdline_with_equals_in_quotes() {
        let result =
            shlex::split("sudo env 'PATH=/path/with spaces' /bin/cmd")
                .unwrap_or_default();
        assert_eq!(
            result,
            vec!["sudo", "env", "PATH=/path/with spaces", "/bin/cmd"]
        );
    }

    #[test]
    fn test_parse_cmdline_multiple_spaces() {
        let result =
            shlex::split("cmd    arg1     arg2").unwrap_or_default();
        assert_eq!(result, vec!["cmd", "arg1", "arg2"]);
    }

    #[test]
    fn test_parse_cmdline_leading_trailing_spaces() {
        let result = shlex::split("  cmd arg1 arg2  ").unwrap_or_default();
        assert_eq!(result, vec!["cmd", "arg1", "arg2"]);
    }

    #[test]
    fn test_parse_cmdline_empty_string() {
        let result = shlex::split("").unwrap_or_default();
        assert_eq!(result, Vec::<String>::default());
    }

    #[test]
    fn test_parse_cmdline_only_spaces() {
        let result = shlex::split("   ").unwrap_or_default();
        assert_eq!(result, Vec::<String>::default());
    }

    #[test]
    fn test_parse_cmdline_realistic_sudo() {
        let cmdline = r"/usr/bin/sudo env 'PATH=/path with spaces' /usr/bin/nh clean all";
        let result = shlex::split(cmdline).unwrap_or_default();
        assert_eq!(
            result,
            vec![
                "/usr/bin/sudo",
                "env",
                "PATH=/path with spaces",
                "/usr/bin/nh",
                "clean",
                "all"
            ]
        );
    }

    #[test]
    fn test_parse_cmdline_escaped_quotes() {
        let result = shlex::split(r#"cmd "arg with \"escaped\" quotes""#)
            .unwrap_or_default();
        assert_eq!(result, vec!["cmd", r#"arg with "escaped" quotes"#]);
    }

    #[test]
    fn test_parse_cmdline_nested_quotes() {
        let result =
            shlex::split(r#"cmd "it's a test""#).unwrap_or_default();
        assert_eq!(result, vec!["cmd", "it's a test"]);
    }

    #[test]
    fn test_parse_cmdline_backslash_outside_quotes() {
        let result =
            shlex::split(r"cmd arg\ with\ space").unwrap_or_default();
        assert_eq!(result, vec!["cmd", "arg with space"]);
    }

    #[test]
    fn test_parse_cmdline_nix_store_paths() {
        let result = shlex::split(
            "/nix/store/abc123-foo/bin/cmd --flag /nix/store/def456-bar",
        )
        .unwrap_or_default();
        assert_eq!(
            result,
            vec![
                "/nix/store/abc123-foo/bin/cmd",
                "--flag",
                "/nix/store/def456-bar"
            ]
        );
    }

    #[test]
    fn test_parse_cmdline_env_vars_in_quotes() {
        let result = shlex::split(r#"env "PATH=$HOME/bin:$PATH" cmd"#)
            .unwrap_or_default();
        assert_eq!(result, vec!["env", "PATH=$HOME/bin:$PATH", "cmd"]);
    }

    #[test]
    fn test_parse_cmdline_unclosed_quote_returns_none() {
        let result = shlex::split("cmd 'unclosed").unwrap_or_default();
        assert_eq!(result, Vec::<String>::default());
    }

    #[test]
    fn test_parse_cmdline_complex_sudo_command() {
        let cmdline = r#"/usr/bin/sudo -E env 'HOME=/root' "PATH=/usr/bin" /usr/bin/nh os switch"#;
        let result = shlex::split(cmdline).unwrap_or_default();
        assert_eq!(
            result,
            vec![
                "/usr/bin/sudo",
                "-E",
                "env",
                "HOME=/root",
                "PATH=/usr/bin",
                "/usr/bin/nh",
                "os",
                "switch"
            ]
        );
    }
}
