use std::ffi::OsStr;
use std::str::FromStr;

use bpaf::{construct, long, positional, parsers::ParseFlag, Parser};
use tracing::warn;

#[derive(Clone, Default, Debug)]
pub enum DiffType {
    /// Display package diff only if the of the
    /// current and the deployed configuration matches.
    #[default]
    Auto,
    /// Always display package diff.
    Always,
    /// Never display package diff.
    Never,
}

impl FromStr for DiffType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            other => Err(format!(
                "expected one of `auto`, `always`, `never`, got `{other}`"
            )),
        }
    }
}

impl std::fmt::Display for DiffType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        })
    }
}

/// clap's boolish value table: `y`, `yes`, `t`, `true`, `on`, `1`
/// (case-insensitive) are true, their counterparts are false, anything else
/// fails.
fn parse_boolish(value: &str) -> Option<bool> {
    match value.to_lowercase().as_str() {
        "y" | "yes" | "t" | "true" | "on" | "1" => Some(true),
        "n" | "no" | "f" | "false" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// Environment fallback that mirrors clap's boolish value parsing.
///
/// Deliberately not bpaf's `switch().env(...)`: that treats any variable value
/// as present, so `NH_ASK=false` would mean "ask".
#[must_use]
pub(crate) fn env_boolish(name: &'static str) -> impl Parser<bool> {
    bpaf::pure(()).parse(move |()| -> std::result::Result<bool, String> {
        std::env::var_os(name).map_or(Ok(false), |value| {
            let value = value.to_string_lossy();
            parse_boolish(&value).map_or_else(
                || {
                    Err(format!(
                        "{name} is set to `{value}`, which is not a \
                         boolean-like value"
                    ))
                },
                Ok,
            )
        })
    })
}

/// Environment fallback accepting only literal `true` and `false`, matching
/// clap's default bool value parser.
#[must_use]
pub(crate) fn env_bool_strict(name: &'static str) -> impl Parser<bool> {
    bpaf::pure(()).parse(move |()| -> std::result::Result<bool, String> {
        match std::env::var_os(name) {
            None => Ok(false),
            Some(value) if value.as_os_str() == OsStr::new("true") => {
                Ok(true)
            }
            Some(value) if value.as_os_str() == OsStr::new("false") => {
                Ok(false)
            }
            Some(value) => Err(format!(
                "{name} is set to `{}`, which is not `true` or `false`",
                value.to_string_lossy()
            )),
        }
    })
}

struct CliAndEnvBool {
    cli: bool,
    from_env: bool,
}

/// Combine a CLI switch with a boolean environment fallback; the CLI switch
/// wins.
#[must_use]
pub(crate) fn switch_or_env(
    flag: ParseFlag<bool>,
    from_env: impl Parser<bool>,
) -> impl Parser<bool> {
    let cli = flag;
    construct!(CliAndEnvBool { cli, from_env })
        .map(|parsed| parsed.cli || parsed.from_env)
}

#[derive(Clone, Debug, Default)]
pub struct NixBuildPassthrough {
    /// Number of concurrent jobs Nix should run.
    pub max_jobs: Option<usize>,

    /// Number of cores Nix should utilize.
    pub cores: Option<usize>,

    /// Logging format used by Nix.
    pub log_format: Option<String>,

    /// Continue building despite encountering errors.
    pub keep_going: bool,

    /// Keep build outputs from failed builds.
    pub keep_failed: bool,

    /// Attempt to build locally if substituters fail.
    pub fallback: bool,

    /// Repair corrupted store paths.
    pub repair: bool,

    /// Explicitly define remote builders.
    pub builders: Option<String>,

    /// Paths to include.
    pub include: Vec<String>,

    /// Print build logs directly to stdout.
    pub print_build_logs: bool,

    /// Display tracebacks on errors.
    pub show_trace: bool,

    /// Accept configuration from flakes.
    pub accept_flake_config: bool,

    /// Refresh flakes to the latest revision.
    pub refresh: bool,

    /// Allow impure builds.
    pub impure: bool,

    /// Build without internet access.
    pub offline: bool,

    /// Prohibit network usage.
    pub no_net: bool,

    /// Recreate the flake.lock file entirely.
    pub recreate_lock_file: bool,

    /// Do not update the flake.lock file.
    pub no_update_lock_file: bool,

    /// Do not write a lock file.
    pub no_write_lock_file: bool,

    /// Do not use registries.
    pub no_use_registries: bool,

    /// Do not use registries (deprecated, use --no-use-registries).
    pub no_registries: bool,

    /// Commit the lock file after updates.
    pub commit_lock_file: bool,

    /// Suppress build output.
    pub no_build_output: bool,

    /// Use substitutes when copying.
    pub use_substitutes: bool,

    /// Output results in JSON format.
    pub json: bool,

    /// Set a Nix configuration option (may be given multiple times).
    pub option: Vec<String>,

    /// Override a specific flake input (may be given multiple times).
    pub override_input: Vec<String>,
}

/// One `--option NAME VALUE` occurrence, parsed as an adjacent group.
fn nix_option_pair() -> impl Parser<(String, String)> {
    let flag = long("option")
        .help("Set a Nix configuration option (may be given multiple times)")
        .req_flag(());
    let name = positional::<String>("NAME");
    let value = positional::<String>("VALUE");
    construct!(flag, name, value)
        .adjacent()
        .map(|((), name, value)| (name, value))
}

/// One `--override-input INPUT FLAKE_URL` occurrence, parsed as an adjacent
/// group.
fn override_input_pair() -> impl Parser<(String, String)> {
    let flag = long("override-input")
        .help("Override a specific flake input (may be given multiple times)")
        .req_flag(());
    let input = positional::<String>("INPUT");
    let flake_url = positional::<String>("FLAKE_URL");
    construct!(flag, input, flake_url)
        .adjacent()
        .map(|((), input, flake_url)| (input, flake_url))
}

/// Flatten repeated `(first, second)` pairs into the flat `Vec<String>` shape
/// the passthrough generator consumes.
fn flatten_pairs(pairs: Vec<(String, String)>) -> Vec<String> {
    let mut flat = Vec::with_capacity(pairs.len() * 2);
    for (first, second) in pairs {
        flat.push(first);
        flat.push(second);
    }
    flat
}

/// CLI parser for [`NixBuildPassthrough`], mirroring the `nix build` options
/// nh forwards.
#[must_use]
pub fn passthrough_cli() -> impl Parser<NixBuildPassthrough> {
    let max_jobs = long("max-jobs")
        .short('j')
        .argument::<usize>("MAX_JOBS")
        .help("Number of concurrent jobs Nix should run")
        .optional();
    let cores = long("cores")
        .argument::<usize>("CORES")
        .help("Number of cores Nix should utilize")
        .optional();
    let log_format = long("log-format")
        .argument::<String>("LOG_FORMAT")
        .help("Logging format used by Nix")
        .optional();
    let keep_going = long("keep-going")
        .short('k')
        .switch()
        .help("Continue building despite encountering errors");
    let keep_failed = long("keep-failed")
        .short('K')
        .switch()
        .help("Keep build outputs from failed builds");
    let fallback = long("fallback")
        .switch()
        .help("Attempt to build locally if substituters fail");
    let repair = long("repair")
        .switch()
        .help("Repair corrupted store paths");
    let builders = long("builders")
        .argument::<String>("BUILDERS")
        .help("Explicitly define remote builders")
        .optional();
    let include = long("include")
        .short('I')
        .argument::<String>("INCLUDE")
        .help("Paths to include")
        .many();
    let print_build_logs = long("print-build-logs")
        .short('L')
        .switch()
        .help("Print build logs directly to stdout");
    let show_trace = long("show-trace")
        .short('t')
        .switch()
        .help("Display tracebacks on errors");
    let accept_flake_config = long("accept-flake-config")
        .switch()
        .help("Accept configuration from flakes");
    let refresh = long("refresh")
        .switch()
        .help("Refresh flakes to the latest revision");
    let impure = long("impure").switch().help("Allow impure builds");
    let offline = long("offline")
        .switch()
        .help("Build without internet access");
    let no_net = long("no-net").switch().help("Prohibit network usage");
    let recreate_lock_file = long("recreate-lock-file")
        .switch()
        .help("Recreate the flake.lock file entirely");
    let no_update_lock_file = long("no-update-lock-file")
        .switch()
        .help("Do not update the flake.lock file");
    let no_write_lock_file = long("no-write-lock-file")
        .switch()
        .help("Do not write a lock file");
    let no_use_registries = long("no-use-registries")
        .switch()
        .help("Do not use registries");
    let no_registries = long("no-registries")
        .switch()
        .help("Do not use registries (deprecated, use --no-use-registries)");
    let commit_lock_file = long("commit-lock-file")
        .switch()
        .help("Commit the lock file after updates");
    let no_build_output = long("no-build-output")
        .short('Q')
        .switch()
        .help("Suppress build output");
    let use_substitutes = long("use-substitutes")
        .switch()
        .help("Use substitutes when copying");
    let json = long("json").switch().help("Output results in JSON format");
    let option = nix_option_pair().many().map(flatten_pairs);
    let override_input = override_input_pair().many().map(flatten_pairs);

    construct!(NixBuildPassthrough {
        max_jobs,
        cores,
        log_format,
        keep_going,
        keep_failed,
        fallback,
        repair,
        builders,
        include,
        print_build_logs,
        show_trace,
        accept_flake_config,
        refresh,
        impure,
        offline,
        no_net,
        recreate_lock_file,
        no_update_lock_file,
        no_write_lock_file,
        no_use_registries,
        no_registries,
        commit_lock_file,
        no_build_output,
        use_substitutes,
        json,
        option,
        override_input,
    })
}

impl NixBuildPassthrough {
    #[must_use]
    pub fn generate_passthrough_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        if let Some(jobs) = self.max_jobs {
            args.push("--max-jobs".into());
            args.push(jobs.to_string());
        }
        if let Some(cores) = self.cores {
            args.push("--cores".into());
            args.push(cores.to_string());
        }
        if let Some(format) = &self.log_format {
            args.push("--log-format".into());
            args.push(format.clone());
        }
        if self.keep_going {
            args.push("--keep-going".into());
        }
        if self.keep_failed {
            args.push("--keep-failed".into());
        }
        if self.fallback {
            args.push("--fallback".into());
        }
        if self.repair {
            args.push("--repair".into());
        }
        if let Some(builders) = &self.builders {
            args.push("--builders".into());
            args.push(builders.clone());
        }
        for inc in &self.include {
            args.push("--include".into());
            args.push(inc.clone());
        }
        if self.print_build_logs {
            args.push("--print-build-logs".into());
        }
        if self.show_trace {
            args.push("--show-trace".into());
        }
        if self.accept_flake_config {
            args.push("--accept-flake-config".into());
        }
        if self.refresh {
            args.push("--refresh".into());
        }
        if self.impure {
            args.push("--impure".into());
        }
        if self.offline {
            args.push("--offline".into());
        }
        if self.no_net {
            args.push("--no-net".into());
        }
        if self.recreate_lock_file {
            args.push("--recreate-lock-file".into());
        }
        if self.no_update_lock_file {
            args.push("--no-update-lock-file".into());
        }
        if self.no_write_lock_file {
            args.push("--no-write-lock-file".into());
        }
        if self.no_use_registries {
            args.push("--no-use-registries".into());
        }
        if self.no_registries {
            warn!(
                "--no-registries is deprecated, use --no-use-registries instead"
            );
            args.push("--no-use-registries".into());
        }
        if self.no_build_output {
            args.push("--quiet".into());
        }
        if self.json {
            args.push("--json".into());
        }
        for [name, value] in self.option.as_chunks::<2>().0 {
            args.push("--option".into());
            args.push(name.clone());
            args.push(value.clone());
        }
        for [input, flake_url] in self.override_input.as_chunks::<2>().0 {
            args.push("--override-input".into());
            args.push(input.clone());
            args.push(flake_url.clone());
        }

        args
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Test assertions")]
mod tests {
    use bpaf::{Args, ParseFailure, Parser as _};

    use super::NixBuildPassthrough;
    use super::parse_boolish;
    use super::passthrough_cli;

    #[test]
    fn no_build_output_maps_to_nix_quiet_flag() {
        let args = NixBuildPassthrough {
            no_build_output: true,
            ..Default::default()
        };

        assert_eq!(args.generate_passthrough_args(), ["--quiet"]);
    }

    #[test]
    fn option_pairs_are_emitted() {
        let args = NixBuildPassthrough {
            option: vec![
                "sandbox".into(),
                "false".into(),
                "cores".into(),
                "4".into(),
            ],
            ..Default::default()
        };

        assert_eq!(
            args.generate_passthrough_args(),
            ["--option", "sandbox", "false", "--option", "cores", "4"]
        );
    }

    #[test]
    fn override_input_pairs_are_emitted() {
        let args = NixBuildPassthrough {
            override_input: vec![
                "nixpkgs".into(),
                "github:NixOS/nixpkgs/nixos-unstable".into(),
            ],
            ..Default::default()
        };

        assert_eq!(
            args.generate_passthrough_args(),
            [
                "--override-input",
                "nixpkgs",
                "github:NixOS/nixpkgs/nixos-unstable"
            ]
        );
    }

    fn parse_passthrough(
        args: &[&str],
    ) -> std::result::Result<NixBuildPassthrough, String> {
        let options = passthrough_cli().to_options();
        options.check_invariants(false);
        options
            .run_inner(Args::from(args).set_name("test"))
            .map_err(ParseFailure::unwrap_stderr)
    }

    #[test]
    fn option_pairs_parse_via_adjacent_group() {
        let args = parse_passthrough(&[
            "--option",
            "sandbox",
            "false",
            "--option",
            "cores",
            "4",
        ])
        .unwrap();

        assert_eq!(args.option, ["sandbox", "false", "cores", "4"]);
        assert_eq!(
            args.generate_passthrough_args(),
            ["--option", "sandbox", "false", "--option", "cores", "4"]
        );
    }

    #[test]
    fn option_pair_without_value_is_rejected() {
        let err = parse_passthrough(&["--option", "sandbox"]).unwrap_err();
        assert!(err.contains("expected `VALUE`"));
    }

    #[test]
    fn hidden_alias_no_registries_still_works() {
        let args = parse_passthrough(&["--no-registries"]).unwrap();
        assert!(args.no_registries);
        assert!(!args.no_use_registries);
        assert_eq!(args.generate_passthrough_args(), ["--no-use-registries"]);
    }

    #[test]
    fn boolish_env_table_matches_clap() {
        for value in ["y", "yes", "t", "true", "on", "1", "TRUE", "Yes"] {
            assert_eq!(parse_boolish(value), Some(true), "value: {value}");
        }
        for value in ["n", "no", "f", "false", "off", "0", "OFF"] {
            assert_eq!(parse_boolish(value), Some(false), "value: {value}");
        }
        assert_eq!(parse_boolish("maybe"), None);
        assert_eq!(parse_boolish(""), None);
    }
}
