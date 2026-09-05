//! Selecting what a nix command should operate on.
//!
//! A [`BuildTarget`] is the parsed form of the installable argument accepted
//! by nix commands: a flake reference, a file, an expression, or a store
//! path, with an attribute path where applicable. Selection consults the CLI
//! first, then the `NH_FLAKE`/`NH_FILE`/`NH_ATTRP` environment variables,
//! then the `/etc/nixos` default.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::ops::Deref;
use std::ops::DerefMut;
use std::path::Path;
use std::path::PathBuf;

use bpaf::{Parser, construct, long, positional};
use rootcause::Result;
use rootcause::report;
use tracing::debug;
use tracing::warn;

use crate::runtime::Env;

/// A parsed Nix attribute path: dot-separated segments with quoting support.
///
/// Segments are stored unquoted. The [`fmt::Display`] implementation renders
/// the canonical form, quoting segments that contain separators.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttrPath(Vec<String>);

impl AttrPath {
    /// Parse an attribute path such as `nixosConfigurations.myhost`.
    ///
    /// Segments containing separators must be quoted with `"` and may use
    /// `\"` and `\\` escapes.
    fn parse(value: &str) -> std::result::Result<Self, &'static str> {
        let mut segments = Vec::new();

        if value.is_empty() {
            return Ok(Self(segments));
        }

        let mut in_quote = false;
        let mut segment = String::new();

        let mut chars = value.chars();
        while let Some(char) = chars.next() {
            match char {
                '.' if !in_quote => {
                    segments.push(std::mem::take(&mut segment));
                }
                '"' => {
                    in_quote = !in_quote;
                }
                '\\' if in_quote => {
                    let escaped = chars.next().ok_or(
                        "contains an incomplete quoted attribute escape",
                    )?;
                    segment.push(escaped);
                }
                _ => segment.push(char),
            }
        }

        segments.push(segment);

        if in_quote {
            return Err("contains an unclosed quoted attribute segment");
        }

        Ok(Self(segments))
    }
}

impl Deref for AttrPath {
    type Target = Vec<String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for AttrPath {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl fmt::Display for AttrPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, segment) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(".")?;
            }

            if segment.is_empty() || segment.contains(['.', '"', '\\']) {
                f.write_str("\"")?;
                for char in segment.chars() {
                    if matches!(char, '"' | '\\') {
                        f.write_str("\\")?;
                    }
                    write!(f, "{char}")?;
                }
                f.write_str("\"")?;
            } else {
                f.write_str(segment)?;
            }
        }

        Ok(())
    }
}

/// What a nix command should operate on.
///
/// This mirrors the installable grammar accepted by the nix CLI: a flake
/// reference, a file, or an expression, each optionally followed by an
/// attribute path, or a bare store path.
#[derive(Debug, Clone)]
pub enum BuildTarget {
    Flake {
        reference: String,
        attribute: AttrPath,
    },
    File {
        path: PathBuf,
        attribute: AttrPath,
    },
    Expression {
        expression: String,
        attribute: AttrPath,
    },
    StorePath(PathBuf),
}

impl BuildTarget {
    /// Name of the target kind, for user-facing messages.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Flake { .. } => "flake",
            Self::File { .. } => "file",
            Self::Expression { .. } => "expression",
            Self::StorePath(_) => "store path",
        }
    }

    /// Render the target as arguments for a nix command.
    #[must_use]
    pub fn to_args(&self) -> Vec<OsString> {
        match self {
            Self::Flake {
                reference,
                attribute,
            } => {
                vec![format!("{reference}#{attribute}").into()]
            }
            Self::File { path, attribute } => {
                vec![
                    OsString::from("--file"),
                    path.clone().into_os_string(),
                    attribute.to_string().into(),
                ]
            }
            Self::Expression {
                expression,
                attribute,
            } => {
                vec![
                    OsString::from("--expr"),
                    OsString::from(expression),
                    attribute.to_string().into(),
                ]
            }
            Self::StorePath(path) => {
                vec![path.clone().into_os_string()]
            }
        }
    }

    /// Preflight explicit local flake references so failures point at the
    /// configuration instead of Nix's parent-directory search.
    fn validate_local_ref(&self) -> Result<()> {
        let Self::Flake { reference, .. } = self else {
            return Ok(());
        };

        let Some(path) = local_flake_path(reference) else {
            return Ok(());
        };

        match resolve_flake_dir(&path) {
            Ok(_) => Ok(()),
            Err(FallbackError::NotFound) => Err(report!(
                "Flake reference `{reference}` points to local path `{}`, but that \
                 path does not exist or does not contain a flake.nix file.\nPass an \
                 existing flake path or update NH_FLAKE if this value came from \
                 the environment.",
                path.display()
            )),
            Err(FallbackError::PermissionDenied(path)) => Err(report!(
                "Permission denied accessing {} while checking flake reference \
                     `{reference}`.",
                path.display()
            )),
            Err(FallbackError::Io(source)) => Err(report!(
                "I/O error checking flake reference `{reference}` at {}: {source}",
                path.display()
            )),
        }
    }
}

/// CLI surface for target selection before resolution.
#[derive(Debug)]
struct CliTarget {
    source: Option<CliSource>,
    positional: Option<String>,
}

/// Hidden `-f`/`--file` and `-E`/`--expr` arguments, mutually exclusive.
#[derive(Debug)]
enum CliSource {
    File(String),
    Expr(String),
}

const TARGET_HELP: &str = "Which target to operate on.
Nix accepts various kinds of installables:

[FLAKEREF[#ATTRPATH]]
    Flake reference with an optional attribute path.
    [env: NH_FLAKE]

-f, --file <FILE> [ATTRPATH]
    Path to file with an optional attribute path.
    [env: NH_FILE]
    [env: NH_ATTRP]

-E, --expr <EXPR> [ATTRPATH]
    Nix expression with an optional attribute path.

[PATH]
    Path or symlink to a /nix/store path";

/// bpaf parser for target selection.
///
/// Accepts a positional target plus the hidden `-f/--file` and `-E/--expr`
/// arguments and resolves the combination at parse time, mirroring the Nix
/// installable grammar. Returns `None` when nothing was supplied; resolution
/// then falls back to the environment and the default.
#[must_use]
pub fn parser() -> impl Parser<Option<BuildTarget>> {
    let file = long("file")
        .short('f')
        .argument::<String>("FILE")
        .hide()
        .map(CliSource::File);
    let expr = long("expr")
        .short('E')
        .argument::<String>("EXPR")
        .hide()
        .map(CliSource::Expr);
    let source = construct!([file, expr]).optional();
    let positional = positional::<String>("TARGET")
        .help(TARGET_HELP)
        .non_strict()
        .optional();
    construct!(CliTarget { source, positional }).parse(resolve_cli)
}

/// Resolve the raw CLI surface into an optional [`BuildTarget`], applying the
/// same precedence as Nix: store path, then `--file`/`--expr`, then flake
/// reference.
fn resolve_cli(
    raw: CliTarget,
) -> std::result::Result<Option<BuildTarget>, String> {
    let CliTarget { source, positional } = raw;

    if let Some(value) = &positional
        && let Ok(path) = fs::canonicalize(value)
        && path.starts_with("/nix/store")
    {
        return Ok(Some(BuildTarget::StorePath(path)));
    }

    if let Some(source) = source {
        let attribute_src = positional.as_deref().unwrap_or_default();
        let attribute = AttrPath::parse(attribute_src)
            .map_err(|err| format!("attribute path {err}"))?;
        return Ok(Some(match source {
            CliSource::File(path) => BuildTarget::File {
                path: PathBuf::from(path),
                attribute,
            },
            CliSource::Expr(expression) => BuildTarget::Expression {
                expression,
                attribute,
            },
        }));
    }

    match positional {
        Some(value) => {
            let (reference, attribute) = parse_flake_reference(&value)
                .map_err(|err| format!("target argument {err}"))?;
            Ok(Some(BuildTarget::Flake {
                reference,
                attribute,
            }))
        }
        None => Ok(None),
    }
}

/// Resolve the target for a command.
///
/// An explicit CLI target wins. Without one, the `NH_FILE`/`NH_ATTRP` and
/// `NH_FLAKE` environment variables are consulted in that order. With
/// neither, the default is a flake at `/etc/nixos`.
///
/// Explicit local flake references are validated before command execution: a
/// supplied local path must point at the directory containing `flake.nix`;
/// `nh` does not let Nix search parent directories for it.
///
/// # Errors
///
/// Returns an error when a configured environment variable is malformed,
/// when a local flake reference does not point at a flake directory, or when
/// no default flake can be found.
pub fn resolve(
    target: Option<BuildTarget>,
    env: &Env,
) -> Result<BuildTarget> {
    let resolved = match target {
        Some(explicit) => explicit,
        None => match env_target(env)? {
            Some(from_env) => from_env,
            None => os_default_target()?,
        },
    };

    resolved.validate_local_ref()?;
    Ok(resolved)
}

/// Target selected by the `NH_FILE`/`NH_ATTRP` and `NH_FLAKE` environment
/// variables, in that order.
///
/// An explicitly set but empty variable is reported as an error instead of
/// being treated as unset, so a misconfigured environment surfaces at
/// resolution rather than silently changing behavior.
fn env_target(env: &Env) -> Result<Option<BuildTarget>> {
    if let Some(file) = env.var("NH_FILE") {
        if file.is_empty() {
            return Err(report!(
                "NH_FILE is empty. Set it to a Nix file path or remove it."
            ));
        }
        let attribute =
            AttrPath::parse(env.var("NH_ATTRP").unwrap_or_default())
                .map_err(|err| report!("NH_ATTRP {err}"))?;
        debug!("Using NH_FILE: {file}");
        return Ok(Some(BuildTarget::File {
            path: PathBuf::from(file),
            attribute,
        }));
    }

    match env.var("NH_FLAKE") {
        Some(value) => {
            debug!("Using NH_FLAKE: {value}");
            let (reference, attribute) = parse_flake_reference(value)
                .map_err(|err| report!("NH_FLAKE {err}"))?;
            Ok(Some(BuildTarget::Flake {
                reference,
                attribute,
            }))
        }
        None => Ok(None),
    }
}

/// Split a flake reference into its reference part and attribute path.
fn parse_flake_reference(
    value: &str,
) -> std::result::Result<(String, AttrPath), &'static str> {
    // CLI targets and NH_FLAKE values share the flakeref grammar. Reject
    // empty references so Nix never turns `""` or `#attr` into an implicit
    // search from the current directory.
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

    Ok((reference.to_owned(), AttrPath::parse(attribute)?))
}

enum FallbackError {
    NotFound,
    PermissionDenied(PathBuf),
    Io(std::io::Error),
}

/// Resolve the directory that actually contains `flake.nix` under `dir`.
///
/// `flake.nix` may be a symlink into another directory (a common setup with
/// dotfiles checkouts), so the flake directory is the parent of the
/// canonical `flake.nix` path.
fn resolve_flake_dir(
    dir: &Path,
) -> std::result::Result<PathBuf, FallbackError> {
    let flake_nix = dir.join("flake.nix");
    let resolved = fs::canonicalize(&flake_nix)
        .map_err(|err| fallback_io_error(err, &flake_nix))?;

    if !resolved.is_file() {
        return Err(FallbackError::NotFound);
    }

    resolved
        .parent()
        .map_or(Err(FallbackError::NotFound), |parent| {
            Ok(parent.to_path_buf())
        })
}

fn fallback_io_error(err: std::io::Error, path: &Path) -> FallbackError {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "io::ErrorKind has many variants; all other errors are wrapped as Io"
    )]
    match err.kind() {
        ErrorKind::NotFound => FallbackError::NotFound,
        ErrorKind::PermissionDenied => {
            FallbackError::PermissionDenied(path.to_path_buf())
        }
        _ => FallbackError::Io(err),
    }
}

/// Filesystem path behind a flake reference that is unmistakably local.
///
/// Bare names like `nixpkgs`, URL/registry-style refs, and parameterized
/// refs such as `path:/repo?dir=nix/flakes` have registry- or
/// scheme-dependent semantics and stay in Nix's hands.
fn local_flake_path(reference: &str) -> Option<PathBuf> {
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

const DEFAULT_HELP_HINT: &str = "See 'man nh' or https://github.com/nix-community/nh for more details.";

/// Default to the system configuration flake when nothing else selects a
/// target.
///
/// # Errors
///
/// Returns an error when `/etc/nixos/flake.nix` cannot be resolved to a
/// flake directory.
fn os_default_target() -> Result<BuildTarget> {
    let default_dir = Path::new("/etc/nixos");

    match resolve_flake_dir(default_dir) {
        Ok(resolved) => {
            warn!(
                "No target was specified, falling back to {}",
                resolved.display()
            );
            let reference = resolved
                .to_str()
                .ok_or_else(|| {
                    report!(
                        "Resolved path {} contains invalid UTF-8",
                        resolved.display()
                    )
                })?
                .to_owned();
            Ok(BuildTarget::Flake {
                reference,
                attribute: AttrPath::default(),
            })
        }
        Err(FallbackError::PermissionDenied(path)) => Err(report!(
            "Permission denied accessing {}.\nPlease either:\n- Pass a flake \
                 path as an argument (e.g., 'nh os switch .')\n- Set the NH_FLAKE \
                 environment variable\n{DEFAULT_HELP_HINT}",
            path.display()
        )),
        Err(FallbackError::Io(source)) => Err(report!(
            "I/O error accessing {}: {source}\n\n{DEFAULT_HELP_HINT}",
            default_dir.display()
        )),
        Err(FallbackError::NotFound) => Err(report!(
            "No target specified and no flake found at {}/flake.nix.\nPlease \
             either:\n- Pass a flake path as an argument (e.g., 'nh os switch \
             .')\n- Set the NH_FLAKE environment variable\n{DEFAULT_HELP_HINT}",
            default_dir.display()
        )),
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, clippy::panic, reason = "Test assertions")]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;

    use bpaf::{Args, ParseFailure, Parser as _};

    use super::AttrPath;
    use super::BuildTarget;
    use super::parser;
    use super::resolve;
    use crate::runtime::Env;

    /// Build an `AttrPath` from unquoted segments.
    fn path(segments: &[&str]) -> AttrPath {
        AttrPath(
            segments
                .iter()
                .map(|segment| (*segment).to_owned())
                .collect(),
        )
    }

    /// An environment snapshot without any target variables set.
    fn empty_env() -> Env {
        Env::from_pairs([] as [(&str, &str); 0])
    }

    #[test]
    fn attrpath_parse() {
        assert_eq!(
            AttrPath::parse("foo.bar").unwrap().to_vec(),
            ["foo", "bar"]
        );
        assert_eq!(
            AttrPath::parse(r#"foo."bar.baz""#).unwrap().to_vec(),
            ["foo", "bar.baz"]
        );
        assert_eq!(
            AttrPath::parse(r#"foo."bar\"baz"."bar\\baz""#)
                .unwrap()
                .to_vec(),
            ["foo", "bar\"baz", "bar\\baz"]
        );
        assert!(AttrPath::parse("").unwrap().is_empty());
        AttrPath::parse(r#"foo."bar"#).unwrap_err();
        AttrPath::parse(r#"foo."bar\"#).unwrap_err();
    }

    #[test]
    fn attrpath_display() {
        assert_eq!(path(&["foo", "bar"]).to_string(), "foo.bar");
        assert_eq!(
            path(&["foo", "bar.baz"]).to_string(),
            r#"foo."bar.baz""#
        );
        assert_eq!(
            path(&["foo", r#"bar"baz"#, r"bar\baz", ""]).to_string(),
            r#"foo."bar\"baz"."bar\\baz"."""#
        );
    }

    #[test]
    fn target_to_args() {
        let flake = BuildTarget::Flake {
            reference: String::from("w"),
            attribute: path(&["x", "y.z"]),
        };
        assert_eq!(flake.to_args(), [r#"w#x."y.z""#]);

        let file = BuildTarget::File {
            path: PathBuf::from("w"),
            attribute: path(&["x", "y.z"]),
        };
        assert_eq!(file.to_args(), ["--file", "w", r#"x."y.z""#]);

        let expression = BuildTarget::Expression {
            expression: String::from("{ pkgs }: pkgs.hello"),
            attribute: AttrPath::default(),
        };
        assert_eq!(
            expression.to_args(),
            ["--expr", "{ pkgs }: pkgs.hello", ""]
        );

        let store =
            BuildTarget::StorePath(PathBuf::from("/nix/store/abc"));
        assert_eq!(store.to_args(), ["/nix/store/abc"]);
    }

    #[test]
    fn cli_target_rejects_empty_reference() {
        let err = parse_target(&[""]).unwrap_err();
        assert!(err.contains("target argument is empty"));
    }

    #[test]
    fn cli_target_rejects_attribute_without_reference() {
        let err = parse_target(&["#fallback"]).unwrap_err();
        assert!(err.contains("missing reference part before `#`"));
    }

    #[test]
    fn cli_file_rejects_malformed_attribute() {
        let err = parse_target(&["--file", "file.nix", r#"foo."bar"#])
            .unwrap_err();
        assert!(err.contains(
            "attribute path contains an unclosed quoted attribute"
        ));
    }

    #[test]
    fn cli_file_and_expr_conflict() {
        let err = parse_target(&["--file", "file.nix", "--expr", "{}"])
            .unwrap_err();
        assert!(err.contains("--expr"));
    }

    #[test]
    fn cli_target_resolves_flake_reference() {
        let Some(BuildTarget::Flake {
            reference,
            attribute,
        }) = parse_target(&["github:user/repo#host"]).unwrap()
        else {
            panic!("Expected a flake target");
        };
        assert_eq!(reference, "github:user/repo");
        assert_eq!(attribute.to_vec(), ["host"]);
    }

    #[test]
    fn cli_target_is_optional() {
        assert!(parse_target(&[]).unwrap().is_none());
    }

    #[test]
    fn resolve_rejects_empty_nh_flake() {
        let env = Env::from_pairs([("NH_FLAKE", "")]);
        let err = resolve(None, &env).unwrap_err().to_string();
        assert!(err.contains("NH_FLAKE is empty"));
    }

    #[test]
    fn resolve_rejects_empty_nh_file() {
        let env = Env::from_pairs([("NH_FILE", "")]);
        let err = resolve(None, &env).unwrap_err().to_string();
        assert!(err.contains("NH_FILE is empty"));
    }

    #[test]
    fn resolve_rejects_env_flake_without_reference_before_attribute() {
        let env = Env::from_pairs([("NH_FLAKE", "#fallback")]);
        let err = resolve(None, &env).unwrap_err().to_string();
        assert!(
            err.contains("NH_FLAKE missing reference part before `#`")
        );
    }

    #[test]
    fn resolve_rejects_malformed_nh_attrp() {
        let env = Env::from_pairs([
            ("NH_FILE", "/path/to/file.nix"),
            ("NH_ATTRP", r#"foo."bar"#),
        ]);
        let err = resolve(None, &env).unwrap_err().to_string();
        assert!(
            err.contains("NH_ATTRP contains an unclosed quoted attribute")
        );
    }

    #[test]
    fn resolve_prefers_nh_file_over_nh_flake() {
        let env = Env::from_pairs([
            ("NH_FILE", "/path/to/file.nix"),
            ("NH_FLAKE", "github:user/repo"),
        ]);
        let BuildTarget::File { path, attribute } =
            resolve(None, &env).unwrap()
        else {
            panic!("Expected a file target");
        };
        assert_eq!(path, PathBuf::from("/path/to/file.nix"));
        assert!(attribute.is_empty());
    }

    #[test]
    fn resolve_env_flake_with_nested_attribute() {
        let env = Env::from_pairs([(
            "NH_FLAKE",
            "github:user/repo#nixosConfigurations.myhost",
        )]);
        let BuildTarget::Flake {
            reference,
            attribute,
        } = resolve(None, &env).unwrap()
        else {
            panic!("Expected a flake target");
        };
        assert_eq!(reference, "github:user/repo");
        assert_eq!(attribute.to_vec(), ["nixosConfigurations", "myhost"]);
    }

    #[test]
    fn resolve_accepts_existing_local_flake_path() {
        let flake_dir = tempfile::tempdir().unwrap();
        fs::write(flake_dir.path().join("flake.nix"), "{}").unwrap();

        let target = BuildTarget::Flake {
            reference: flake_dir.path().to_string_lossy().into_owned(),
            attribute: AttrPath::default(),
        };

        let resolved = resolve(Some(target), &empty_env()).unwrap();
        assert_eq!(
            resolved.to_args(),
            [OsString::from(format!("{}#", flake_dir.path().display()))]
        );
    }

    #[test]
    fn resolve_rejects_missing_absolute_path() {
        let parent = tempfile::tempdir().unwrap();
        let missing_path = parent.path().join("missing-flake");
        assert!(!missing_path.exists());

        let target = BuildTarget::Flake {
            reference: missing_path.to_string_lossy().into_owned(),
            attribute: AttrPath::default(),
        };

        let err =
            resolve(Some(target), &empty_env()).unwrap_err().to_string();
        assert!(err.contains("Flake reference"));
        assert!(
            err.contains("does not exist or does not contain a flake.nix")
        );
        assert!(err.contains("NH_FLAKE"));
    }

    #[test]
    fn resolve_rejects_existing_dir_without_flake_nix() {
        let dir = tempfile::tempdir().unwrap();

        let target = BuildTarget::Flake {
            reference: dir.path().to_string_lossy().into_owned(),
            attribute: AttrPath::default(),
        };

        let err =
            resolve(Some(target), &empty_env()).unwrap_err().to_string();
        assert!(
            err.contains("does not exist or does not contain a flake.nix")
        );
    }

    #[test]
    fn resolve_rejects_subdir_inside_flake() {
        let flake_dir = tempfile::tempdir().unwrap();
        fs::write(flake_dir.path().join("flake.nix"), "{}").unwrap();
        let subdir = flake_dir.path().join("modules");
        fs::create_dir_all(&subdir).unwrap();

        let target = BuildTarget::Flake {
            reference: subdir.to_string_lossy().into_owned(),
            attribute: AttrPath::default(),
        };

        let err =
            resolve(Some(target), &empty_env()).unwrap_err().to_string();
        assert!(
            err.contains("does not exist or does not contain a flake.nix")
        );
    }

    #[test]
    fn resolve_rejects_missing_path_scheme() {
        let parent = tempfile::tempdir().unwrap();
        let missing_path = parent.path().join("missing-flake");
        assert!(!missing_path.exists());

        let target = BuildTarget::Flake {
            reference: format!("path:{}", missing_path.display()),
            attribute: AttrPath::default(),
        };

        let err =
            resolve(Some(target), &empty_env()).unwrap_err().to_string();
        assert!(err.contains("NH_FLAKE"));
    }

    #[test]
    fn resolve_defers_parameterized_local_flake_refs_to_nix() {
        let source_dir = tempfile::tempdir().unwrap();

        for reference in [
            format!("path:{}?lastModified=1", source_dir.path().display()),
            format!("path:{}?dir=nix/flakes", source_dir.path().display()),
            format!("{}?submodules=1", source_dir.path().display()),
        ] {
            let target = BuildTarget::Flake {
                reference: reference.clone(),
                attribute: AttrPath::default(),
            };

            let resolved = resolve(Some(target), &empty_env()).unwrap();
            assert_eq!(
                resolved.to_args(),
                [OsString::from(format!("{reference}#"))]
            );
        }
    }

    #[test]
    fn resolve_ignores_registry_and_url_refs() {
        for reference in ["nixpkgs", "github:NixOS/nixpkgs"] {
            let target = BuildTarget::Flake {
                reference: reference.to_owned(),
                attribute: AttrPath::default(),
            };

            resolve(Some(target), &empty_env()).unwrap();
        }
    }

    /// Parse the target CLI surface, returning the rendered error message on
    /// parse failure.
    fn parse_target(
        args: &[&str],
    ) -> std::result::Result<Option<BuildTarget>, String> {
        let options = parser().to_options();
        options.check_invariants(false);
        options
            .run_inner(Args::from(args).set_name("test"))
            .map_err(ParseFailure::unwrap_stderr)
    }
}
