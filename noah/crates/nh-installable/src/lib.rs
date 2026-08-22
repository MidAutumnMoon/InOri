use std::{
    fs,
    path::{Path, PathBuf},
};

use clap::{
    Arg, ArgAction, Args, FromArgMatches, ValueHint, error::ErrorKind,
};
use tracing::debug;
use yansi::{Color, Paint};

#[cfg(test)]
mod tests;

// Reference: https://nix.dev/manual/nix/2.18/command-ref/new-cli/nix

/// Flake selection settings supplied by the application.
#[derive(Debug, Clone, Default)]
pub struct FlakeConfig {
    /// `NH_OS_FLAKE` — OS-specific flake reference.
    pub os_flake: Option<String>,
    /// `NH_FLAKE` — generic flake reference.
    pub flake: Option<String>,
    /// `NH_FILE` — path to a Nix file.
    pub file: Option<String>,
    /// `NH_ATTRP` — attribute path for file-based installables.
    pub attrp: String,
}

#[derive(Debug, Clone)]
pub enum InstallableArgs {
    Specified(Installable),
    Unspecified,
}

enum EnvInstallableSource {
    SpecificFlake {
        env_var: &'static str,
        value: String,
    },
    File {
        path: String,
        attribute: String,
    },
    GenericFlake(String),
}

impl EnvInstallableSource {
    const fn uses_flakes(&self) -> bool {
        match self {
            Self::SpecificFlake { value, .. }
            | Self::GenericFlake(value) => !value.is_empty(),
            Self::File { .. } => false,
        }
    }

    fn into_installable(self) -> rootcause::Result<Installable> {
        match self {
            Self::SpecificFlake { env_var, value } => {
                debug!("Using {env_var}: {value}");
                flake_from_env_var(env_var, &value)
            }
            Self::File { path, attribute } => {
                debug!("Using NH_FILE: {path}");
                Ok(Installable::File {
                    path: PathBuf::from(path),
                    attribute: parse_attribute(&attribute).map_err(
                        |err| rootcause::report!("NH_ATTRP {err}"),
                    )?,
                })
            }
            Self::GenericFlake(value) => {
                debug!("Using NH_FLAKE: {value}");
                flake_from_env_var("NH_FLAKE", &value)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum Installable {
    Flake {
        reference: String,
        attribute: Vec<String>,
    },
    File {
        path: PathBuf,
        attribute: Vec<String>,
    },
    Store {
        path: PathBuf,
    },
    Expression {
        expression: String,
        attribute: Vec<String>,
    },
}

impl FromArgMatches for InstallableArgs {
    fn from_arg_matches(
        matches: &clap::ArgMatches,
    ) -> Result<Self, clap::Error> {
        let mut matches = matches.clone();
        Self::from_arg_matches_mut(&mut matches)
    }

    fn from_arg_matches_mut(
        matches: &mut clap::ArgMatches,
    ) -> Result<Self, clap::Error> {
        let installable = matches.get_one::<String>("installable");
        let file = matches.get_one::<String>("file");
        let expr = matches.get_one::<String>("expr");

        if let Some(i) = installable {
            let canonical = fs::canonicalize(i);

            if let Ok(path) = canonical
                && path.starts_with("/nix/store")
            {
                return Ok(Self::Specified(Installable::Store {
                    path: path,
                }));
            }
        }

        if let Some(file_path) = file {
            let attribute =
                parse_attribute(installable.map_or("", String::as_str))
                    .map_err(|err| {
                        clap::Error::raw(
                            ErrorKind::ValueValidation,
                            format!("attribute path {err}"),
                        )
                    })?;
            return Ok(Self::Specified(Installable::File {
                path: PathBuf::from(file_path),
                attribute,
            }));
        }

        if let Some(expression) = expr {
            let attribute =
                parse_attribute(installable.map_or("", String::as_str))
                    .map_err(|err| {
                        clap::Error::raw(
                            ErrorKind::ValueValidation,
                            format!("attribute path {err}"),
                        )
                    })?;
            return Ok(Self::Specified(Installable::Expression {
                expression: expression.clone(),
                attribute,
            }));
        }

        if let Some(i) = installable {
            let (reference, attribute) = parse_flake_reference(i)
                .map_err(|err| {
                    clap::Error::raw(
                        ErrorKind::ValueValidation,
                        format!("installable argument {err}"),
                    )
                })?;
            return Ok(Self::Specified(Installable::Flake {
                reference,
                attribute,
            }));
        }

        Ok(Self::Unspecified)
    }

    fn update_from_arg_matches(
        &mut self,
        matches: &clap::ArgMatches,
    ) -> Result<(), clap::Error> {
        *self = Self::from_arg_matches(matches)?;
        Ok(())
    }
}

impl Args for InstallableArgs {
    fn augment_args(cmd: clap::Command) -> clap::Command {
        cmd.arg(
            Arg::new("file")
                .short('f')
                .long("file")
                .action(ArgAction::Set)
                .hide(true),
        )
        .arg(
            Arg::new("expr")
                .short('E')
                .long("expr")
                .conflicts_with("file")
                .hide(true)
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("installable")
                .action(ArgAction::Set)
                .value_hint(ValueHint::AnyPath)
                .value_name("INSTALLABLE")
                .help("Which installable to use")
                .long_help(format!(
                    "Which installable to use.
Nix accepts various kinds of installables:

[FLAKEREF[#ATTRPATH]]
    Flake reference with an optional attribute path.
    [env: NH_FLAKE]
    [env: NH_OS_FLAKE]

{}, {} <FILE> [ATTRPATH]
    Path to file with an optional attribute path.
    [env: NH_FILE]
    [env: NH_ATTRP]

{}, {} <EXPR> [ATTRPATH]
    Nix expression with an optional attribute path.

[PATH]
    Path or symlink to a /nix/store path
",
                    Paint::new("-f").fg(Color::Yellow),
                    Paint::new("--file").fg(Color::Yellow),
                    Paint::new("-e").fg(Color::Yellow),
                    Paint::new("--expr").fg(Color::Yellow),
                )),
        )
    }

    fn augment_args_for_update(cmd: clap::Command) -> clap::Command {
        Self::augment_args(cmd)
    }
}

fn parse_attribute(attribute: &str) -> Result<Vec<String>, &'static str> {
    let mut res = Vec::new();

    if attribute.is_empty() {
        return Ok(res);
    }

    let mut in_quote = false;
    let mut elem = String::new();

    let mut chars = attribute.chars();
    while let Some(char) = chars.next() {
        match char {
            '.' => {
                if in_quote {
                    elem.push(char);
                } else {
                    res.push(elem.clone());
                    elem = String::new();
                }
            }
            '"' => {
                in_quote = !in_quote;
            }
            '\\' if in_quote => {
                let escaped = chars.next().ok_or(
                    "contains an incomplete quoted attribute escape",
                )?;
                elem.push(escaped);
            }
            _ => elem.push(char),
        }
    }

    res.push(elem);

    if in_quote {
        return Err("contains an unclosed quoted attribute segment");
    }

    Ok(res)
}

fn parse_flake_reference(
    value: &str,
) -> Result<(String, Vec<String>), &'static str> {
    // CLI installables and NH_*_FLAKE values share the same flakeref grammar.
    // Reject empty references here so Nix never turns `""` or `#attr` into an
    // implicit search from the current directory.
    if value.is_empty() {
        return Err("is empty. Set it to a flake reference or remove it.");
    }

    let (reference, attribute) = value
        .split_once('#')
        .map_or((value, ""), |(reference, attribute)| {
            (reference, attribute)
        });

    if reference.is_empty() {
        return Err("missing reference part before `#`");
    }

    let attribute = parse_attribute(attribute)?;
    Ok((reference.to_owned(), attribute))
}

impl InstallableArgs {
    /// Returns whether the parsed CLI input or non-empty flake environment
    /// variables select flake mode for the command context.
    #[must_use]
    pub fn uses_flakes(&self, config: &FlakeConfig) -> bool {
        // Empty flake env vars are invalid inputs. Do not count them as feature
        // requirements here; resolution reports the targeted validation error.
        match self {
            Self::Specified(Installable::Flake { .. }) => true,
            Self::Specified(_) => false,
            Self::Unspecified => env_installable_source(config)
                .is_some_and(|source| source.uses_flakes()),
        }
    }

    /// Resolves an installable from the CLI value or environment.
    ///
    /// If an installable was supplied on the CLI, returns it as-is. Otherwise,
    /// checks env vars in priority order:
    /// - `NH_OS_FLAKE`
    /// - `NH_FILE`, with `NH_ATTRP` as the optional attribute path
    /// - `NH_FLAKE`
    ///
    /// Returns `None` when no installable environment variable is set.
    ///
    /// # Errors
    ///
    /// Returns an error when a configured flake environment variable is
    /// malformed.
    fn resolve(
        self,
        config: &FlakeConfig,
    ) -> rootcause::Result<Option<Installable>> {
        match self {
            Self::Unspecified => env_installable_source(config)
                .map(EnvInstallableSource::into_installable)
                .transpose(),
            Self::Specified(installable) => Ok(Some(installable)),
        }
    }

    /// Resolve an installable and fall back to the default when the installable
    /// is unspecified.
    ///
    /// Explicit local flake references are validated before command execution. A
    /// supplied local path must point at the directory containing `flake.nix`;
    /// `nh` does not let Nix search parent directories for it.
    ///
    /// # Errors
    ///
    /// Returns an error when environment resolution fails, when a local flake
    /// reference does not point at a flake directory, or when no default
    /// installable can be found.
    pub fn resolve_or_default(
        self,
        config: &FlakeConfig,
    ) -> rootcause::Result<Installable> {
        let Some(installable) = self.resolve(config)? else {
            return default_installable_for();
        };

        installable.validate_local_flake_ref()?;
        Ok(installable)
    }
}

fn env_installable_source(
    config: &FlakeConfig,
) -> Option<EnvInstallableSource> {
    if let Some(value) = &config.os_flake {
        return Some(EnvInstallableSource::SpecificFlake {
            env_var: "NH_OS_FLAKE",
            value: value.clone(),
        });
    }

    if let Some(path) = &config.file {
        return Some(EnvInstallableSource::File {
            path: path.clone(),
            attribute: config.attrp.clone(),
        });
    }

    if let Some(value) = &config.flake {
        return Some(EnvInstallableSource::GenericFlake(value.clone()));
    }

    None
}

fn default_installable_for() -> rootcause::Result<Installable> {
    try_find_default_for_os()
}

fn flake_from_env_var(
    name: &str,
    value: &str,
) -> rootcause::Result<Installable> {
    let (reference, attribute) = parse_flake_reference(value)
        .map_err(|err| rootcause::report!("{name} {err}"))?;
    Ok(Installable::Flake {
        reference,
        attribute,
    })
}

impl Installable {
    #[must_use]
    pub fn to_args(&self) -> Vec<String> {
        let mut res = Vec::new();
        match self {
            Self::Flake {
                reference,
                attribute,
            } => {
                res.push(format!(
                    "{reference}#{}",
                    join_attribute(attribute)
                ));
            }
            Self::File { path, attribute } => {
                if let Some(path_str) = path.to_str() {
                    res.push(String::from("--file"));
                    res.push(path_str.to_owned());
                    res.push(join_attribute(attribute));
                } else {
                    // Return empty args if path contains invalid UTF-8
                    return Vec::new();
                }
            }
            Self::Expression {
                expression,
                attribute,
            } => {
                res.push(String::from("--expr"));
                res.push(expression.clone());
                res.push(join_attribute(attribute));
            }
            Self::Store { path } => {
                if let Some(path_str) = path.to_str() {
                    res.push(path_str.to_owned());
                } else {
                    // Return empty args if path contains invalid UTF-8
                    return Vec::new();
                }
            }
        }

        res
    }

    fn validate_local_flake_ref(&self) -> rootcause::Result<()> {
        let Self::Flake { reference, .. } = self else {
            return Ok(());
        };

        let Some(path) = local_flake_reference_path(reference) else {
            return Ok(());
        };

        // For explicit local refs, fail before invoking Nix so the error points at
        // the bad configuration instead of Nix's parent-directory search.
        match resolve_fallback_flake_dir(&path) {
            Ok(_) => Ok(()),
            Err(FallbackError::NotFound) => Err(rootcause::report!(
                "Flake reference `{}` points to local path `{}`, but that path does \
           not exist or does not contain a flake.nix file.\nPass an existing \
           flake path or update NH_FLAKE/NH_OS_FLAKE if this value came from \
           the environment.",
                reference,
                path.display()
            )),
            Err(FallbackError::PermissionDenied(path)) => {
                Err(rootcause::report!(
                    "Permission denied accessing {} while checking flake reference `{}`.",
                    path.display(),
                    reference
                ))
            }
            Err(FallbackError::Io(source)) => Err(rootcause::report!(
                "I/O error checking flake reference `{}` at {}: {}",
                reference,
                path.display(),
                source
            )),
        }
    }

    #[must_use]
    pub const fn str_kind(&self) -> &str {
        match self {
            Self::Flake { .. } => "flake",
            Self::File { .. } => "file",
            Self::Store { .. } => "store path",
            Self::Expression { .. } => "expression",
        }
    }
}

fn join_attribute<I>(attribute: I) -> String
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut res = String::new();
    let mut first = true;
    for elem in attribute {
        if first {
            first = false;
        } else {
            res.push('.');
        }

        let segment = elem.as_ref();

        if segment.is_empty() || segment.contains(['.', '"', '\\']) {
            res.push('"');
            for char in segment.chars() {
                match char {
                    '"' | '\\' => {
                        res.push('\\');
                        res.push(char);
                    }
                    _ => res.push(char),
                }
            }
            res.push('"');
        } else {
            res.push_str(segment);
        }
    }

    res
}

fn local_flake_reference_path(reference: &str) -> Option<PathBuf> {
    // Only preflight references that are unmistakably filesystem paths. Bare
    // names like `nixpkgs`, plus URL/registry-style refs, stay in Nix's hands.
    // Parameterized local flake references such as `path:/repo?dir=nix/flakes`
    // and `./nix?submodules=1` have scheme- and repository-dependent semantics,
    // so we leave their interpretation to Nix as well.
    if reference.contains('?') {
        return None;
    }

    if let Some(path) = reference.strip_prefix("path:") {
        return Some(PathBuf::from(path));
    }

    let path = Path::new(reference);

    if path.is_absolute()
        || matches!(path.to_str(), Some("." | ".."))
        || path.starts_with("./")
        || path.starts_with("../")
    {
        return Some(path.to_path_buf());
    }

    None
}
enum FallbackError {
    NotFound,
    PermissionDenied(PathBuf),
    Io(std::io::Error),
}

/// Resolves a fallback flake directory.
///
/// # Returns
///
/// The resolved path to use as a flake reference. This handles three cases:
///
/// 1. Directory is a symlink -> returns the resolved directory path
/// 2. Directory is real but flake.nix is a symlink → returns the parent
///    directory of the resolved flake.nix
/// 3. Both are real -> returns the original directory
///
/// # Errors
///
/// Returns an error if:
///
/// - The directory does not exist
/// - The directory exists but does not contain a flake.nix file
/// - Permission is denied accessing the directory or flake.nix
/// - Any other I/O error occurs
fn resolve_fallback_flake_dir(
    dir: &std::path::Path,
) -> Result<PathBuf, FallbackError> {
    use std::io::ErrorKind;

    // Check if the directory itself is a symlink
    let dir_is_symlink = dir.is_symlink();

    // Resolve the directory path
    let resolved_dir = match fs::canonicalize(dir) {
        Ok(path) => path,
        Err(err) => {
            return match err.kind() {
                ErrorKind::NotFound => Err(FallbackError::NotFound),
                ErrorKind::PermissionDenied => {
                    Err(FallbackError::PermissionDenied(dir.to_path_buf()))
                }
                _ => Err(FallbackError::Io(err)),
            };
        }
    };

    // If the directory itself was a symlink, use the resolved directory
    if dir_is_symlink {
        let flake_path = resolved_dir.join("flake.nix");
        return match fs::metadata(&flake_path) {
            Ok(metadata) if metadata.is_file() => Ok(resolved_dir),
            Ok(_) => Err(FallbackError::NotFound),
            Err(err) => match err.kind() {
                ErrorKind::NotFound => Err(FallbackError::NotFound),
                ErrorKind::PermissionDenied => {
                    Err(FallbackError::PermissionDenied(flake_path))
                }
                _ => Err(FallbackError::Io(err)),
            },
        };
    }

    // Directory is real, check flake.nix
    let flake_path = resolved_dir.join("flake.nix");

    // Check if flake.nix is a symlink
    if flake_path.is_symlink() {
        // Resolve the symlink to get the actual flake.nix location
        match fs::canonicalize(&flake_path) {
            Ok(resolved_flake) => {
                // Use the parent directory of the resolved flake.nix
                resolved_flake
                    .parent()
                    .map_or(Err(FallbackError::NotFound), |parent| {
                        Ok(parent.to_path_buf())
                    })
            }
            Err(err) => match err.kind() {
                ErrorKind::NotFound => Err(FallbackError::NotFound),
                ErrorKind::PermissionDenied => {
                    Err(FallbackError::PermissionDenied(flake_path))
                }
                _ => Err(FallbackError::Io(err)),
            },
        }
    } else {
        // flake.nix is a real file, check it exists
        match fs::metadata(&flake_path) {
            Ok(metadata) if metadata.is_file() => Ok(resolved_dir),
            Ok(_) => Err(FallbackError::NotFound),
            Err(err) => match err.kind() {
                ErrorKind::NotFound => Err(FallbackError::NotFound),
                ErrorKind::PermissionDenied => {
                    Err(FallbackError::PermissionDenied(flake_path))
                }
                _ => Err(FallbackError::Io(err)),
            },
        }
    }
}

const FALLBACK_HELP_HINT: &str = "See 'man nh' or https://github.com/nix-community/nh for more details.";

/// Attempts to find a default installable for `NixOS` builds.
///
/// Checks if `/etc/nixos/flake.nix` exists and returns a flake installable
/// pointing to it if found. If the directory is a symlink, it is resolved to
/// its canonical path. Otherwise, returns an error with instructions on how to
/// specify an installable.
///
/// # Errors
///
/// Returns an error if:
///
/// - No flake is found at `/etc/nixos/flake.nix`
/// - Permission is denied accessing the path
/// - The resolved path contains invalid UTF-8
fn try_find_default_for_os() -> rootcause::Result<Installable> {
    use tracing::warn;

    let default_dir = std::path::Path::new("/etc/nixos");

    match resolve_fallback_flake_dir(default_dir) {
        Ok(resolved) => {
            warn!(
                "No installable was specified, falling back to {}",
                resolved.display()
            );
            Ok(Installable::Flake {
                reference: resolved
                    .to_str()
                    .ok_or_else(|| {
                        rootcause::report!(
                            "Resolved path {} contains invalid UTF-8",
                            resolved.display()
                        )
                    })?
                    .to_owned(),
                attribute: vec![],
            })
        }
        Err(FallbackError::PermissionDenied(path)) => {
            Err(rootcause::report!(
                "Permission denied accessing {}.\nPlease either:\n- Pass a flake path \
         as an argument (e.g., 'nh os switch .')\n- Set the NH_FLAKE \
         environment variable\n- Set the NH_OS_FLAKE environment \
         variable\n\n{}",
                path.display(),
                FALLBACK_HELP_HINT
            ))
        }
        Err(FallbackError::Io(err)) => Err(rootcause::report!(
            "I/O error accessing {}: {}\n\n{}",
            default_dir.display(),
            err,
            FALLBACK_HELP_HINT
        )),
        Err(FallbackError::NotFound) => Err(rootcause::report!(
            "No installable specified and no flake found at {}/flake.nix.\nPlease \
         either:\n- Pass a flake path as an argument (e.g., 'nh os switch \
         .')\n- Set the NH_FLAKE environment variable\n- Set the NH_OS_FLAKE \
         environment variable\n\n{}",
            default_dir.display(),
            FALLBACK_HELP_HINT
        )),
    }
}
