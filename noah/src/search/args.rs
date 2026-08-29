use std::str::FromStr;

use bpaf::{construct, long, positional, Parser};

use crate::args::env_boolish;
use crate::args::switch_or_env;

const DEFAULT_LIMIT: u64 = 30;
const DEFAULT_BACKEND_FALLBACKS: u32 = 1;

#[derive(Clone, Debug)]
pub struct Search {
    /// Number of search results to display.
    pub limit: u64,

    /// Show supported platforms for each package.
    pub platforms: bool,

    /// Backend index version to query on search.nixos.org. Defaults to the
    /// version bundled with nh.
    pub backend_version: Option<u32>,

    /// Number of newer index versions to try when the requested version is
    /// outdated (missing on the backend).
    pub backend_fallbacks: u32,

    /// Output results as JSON.
    pub json: bool,

    /// Default search mode used when no subcommand is given.
    /// Accepts `packages` or `options`.
    pub default_search: SearchKind,

    pub mode: Option<SearchMode>,

    /// Query shorthand: equivalent to `nh search packages <query>` or
    /// `nh search options <query>` depending on `--default-search`.
    pub query: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum SearchMode {
    /// Search packages via search.nixos.org.
    Packages(Packages),
    /// Search NixOS options via search.nixos.org.
    Options(Options),
}

#[derive(Clone, Debug)]
pub struct Packages {
    /// Name of the package to search.
    pub query: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Name of the option to search.
    pub query: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum SearchKind {
    /// Search packages (default).
    #[default]
    Packages,
    /// Search NixOS options.
    Options,
}

impl FromStr for SearchKind {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "packages" => Ok(Self::Packages),
            "options" => Ok(Self::Options),
            other => Err(format!(
                "expected one of `packages`, `options`, got `{other}`"
            )),
        }
    }
}

impl std::fmt::Display for SearchKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Packages => "packages",
            Self::Options => "options",
        })
    }
}

/// Flag group shared by the search command and its mode subcommands; the
/// group is declared once, before the mode subcommand, so bpaf accepts the
/// flags on either side of `packages`/`options`.
#[derive(Clone, Debug)]
struct SearchFlags {
    limit: u64,
    platforms: bool,
    backend_version: Option<u32>,
    backend_fallbacks: u32,
    json: bool,
    default_search: SearchKind,
}

#[must_use]
fn search_flags() -> impl Parser<SearchFlags> {
    let limit = long("limit")
        .short('l')
        .argument::<u64>("LIMIT")
        .help("Number of search results to display")
        .fallback(DEFAULT_LIMIT)
        .display_fallback();
    let platforms = switch_or_env(
        long("platforms")
            .short('P')
            .help("Show supported platforms for each package")
            .switch(),
        env_boolish("NH_SEARCH_PLATFORM"),
    );
    let backend_version = long("backend-version")
        .env("NH_SEARCH_BACKEND_VERSION")
        .argument::<u32>("VERSION")
        .help(
            "Backend index version to query on search.nixos.org. Defaults \
             to the version bundled with nh",
        )
        .optional();
    let backend_fallbacks = long("backend-version-fallbacks")
        .env("NH_SEARCH_BACKEND_FALLBACKS")
        .argument::<u32>("COUNT")
        .help(
            "Number of newer index versions to try when the requested \
             version is outdated (missing on the backend)",
        )
        .fallback(DEFAULT_BACKEND_FALLBACKS)
        .display_fallback();
    let json = switch_or_env(
        long("json").short('j').help("Output results as JSON").switch(),
        env_boolish("NH_SEARCH_JSON"),
    );
    let default_search = long("default-search")
        .env("NH_DEFAULT_SEARCH")
        .argument::<SearchKind>("MODE")
        .help("Default search mode used when no subcommand is given. Accepts `packages` or `options`")
        .fallback(SearchKind::Packages)
        .display_fallback();

    construct!(SearchFlags {
        limit,
        platforms,
        backend_version,
        backend_fallbacks,
        json,
        default_search,
    })
}

/// CLI parser for the `packages` mode subcommand.
#[must_use]
fn packages_cli() -> impl Parser<SearchMode> {
    let query = positional::<String>("QUERY").help("Name of the package to search").many();
    construct!(Packages { query })
        .to_options()
        .descr("Search packages via search.nixos.org.")
        .command("packages")
        .map(SearchMode::Packages)
}

/// CLI parser for the `options` mode subcommand.
#[must_use]
fn options_cli() -> impl Parser<SearchMode> {
    let query = positional::<String>("QUERY").help("Name of the option to search").many();
    construct!(Options { query })
        .to_options()
        .descr("Search NixOS options via search.nixos.org.")
        .command("options")
        .map(SearchMode::Options)
}

#[derive(Clone, Debug)]
struct SearchWithMode {
    flags: SearchFlags,
    mode: SearchMode,
}

#[derive(Clone, Debug)]
struct SearchWithoutMode {
    flags: SearchFlags,
    query: Vec<String>,
}

/// Reject mode subcommands without query terms. The emptiness check lives
/// after the alternatives so that a missing query is an error instead of
/// falling back to searching for the word `packages`/`options` verbatim.
fn check_mode_query(search: Search) -> std::result::Result<Search, String> {
    let empty = match &search.mode {
        Some(SearchMode::Packages(args)) => args.query.is_empty(),
        Some(SearchMode::Options(args)) => args.query.is_empty(),
        None => false,
    };
    if empty {
        return Err(String::from(
            "expected at least one query term after `packages`/`options`",
        ));
    }
    Ok(search)
}

/// CLI parser for [`Search`]. The mode subcommand is optional; without it the
/// positional query words select the mode via `--default-search`.
#[must_use]
pub fn search_cli() -> impl Parser<Search> {
    let with_mode = {
        let flags = search_flags();
        let mode = construct!([packages_cli(), options_cli()]);
        construct!(SearchWithMode { flags, mode }).map(
            |SearchWithMode {
                 flags: parsed_flags,
                 mode: parsed_mode,
             }| Search {
                limit: parsed_flags.limit,
                platforms: parsed_flags.platforms,
                backend_version: parsed_flags.backend_version,
                backend_fallbacks: parsed_flags.backend_fallbacks,
                json: parsed_flags.json,
                default_search: parsed_flags.default_search,
                mode: Some(parsed_mode),
                query: Vec::new(),
            },
        )
    };
    let without_mode = {
        let flags = search_flags();
        let query = positional::<String>("QUERY").help(
            "Query shorthand: equivalent to `nh search packages <query>` or \
             `nh search options <query>` depending on --default-search",
        );
        let query = query.many();
        construct!(SearchWithoutMode { flags, query }).map(
            |SearchWithoutMode {
                 flags: parsed_flags,
                 query: parsed_query,
             }| Search {
                limit: parsed_flags.limit,
                platforms: parsed_flags.platforms,
                backend_version: parsed_flags.backend_version,
                backend_fallbacks: parsed_flags.backend_fallbacks,
                json: parsed_flags.json,
                default_search: parsed_flags.default_search,
                mode: None,
                query: parsed_query,
            },
        )
    };

    construct!([with_mode, without_mode]).parse(check_mode_query)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, clippy::panic, reason = "Test assertions")]
mod tests {
    use std::assert_matches;

    use bpaf::{Args, ParseFailure, Parser as _};

    use super::{Search, SearchKind, SearchMode, search_cli};

    fn parse_search(args: &[&str]) -> std::result::Result<Search, String> {
        let options = search_cli().to_options();
        options.check_invariants(false);
        options
            .run_inner(Args::from(args).set_name("search"))
            .map_err(ParseFailure::unwrap_stderr)
    }

    #[test]
    fn global_flags_work_on_both_sides_of_subcommand() {
        let args = parse_search(&[
            "--limit",
            "5",
            "--backend-version",
            "51",
            "packages",
            "hello",
            "--platforms",
            "--backend-version-fallbacks",
            "3",
            "--json",
        ])
        .unwrap();

        assert_eq!(args.limit, 5);
        assert!(args.platforms);
        assert!(args.json);
        assert_eq!(args.backend_version, Some(51));
        assert_eq!(args.backend_fallbacks, 3);
        match args.mode {
            Some(SearchMode::Packages(packages)) => {
                assert_eq!(packages.query, ["hello"]);
            }
            other => {
                panic!("expected packages mode, got {other:?}");
            }
        }
    }

    #[test]
    fn shorthand_flags_parse_after_query() {
        let args = parse_search(&[
            "hello",
            "--limit",
            "5",
            "--platforms",
            "--default-search",
            "packages",
        ])
        .unwrap();

        assert_eq!(args.limit, 5);
        assert!(args.platforms);
        assert_matches!(args.default_search, SearchKind::Packages);
        assert_eq!(args.query, ["hello"]);
        assert!(args.mode.is_none());
    }

    #[test]
    fn default_search_parses_after_shorthand_query() {
        let args =
            parse_search(&["hello", "--default-search", "options"]).unwrap();

        assert_matches!(args.default_search, SearchKind::Options);
        assert_eq!(args.query, ["hello"]);
        assert!(args.mode.is_none());
    }

    #[test]
    fn backend_flags_have_shared_defaults() {
        let args = parse_search(&["options", "hello"]).unwrap();
        assert_eq!(args.backend_version, None);
        assert_eq!(args.backend_fallbacks, 1);
    }

    #[test]
    fn options_preserve_platform_flag_for_runtime_validation() {
        let args = parse_search(&["options", "hello", "--platforms"]).unwrap();
        assert!(args.platforms);
        assert_matches!(args.mode, Some(SearchMode::Options(_)));
    }

    #[test]
    fn subcommand_requires_query() {
        let err = parse_search(&["packages"]).unwrap_err();
        assert!(err.contains("expected at least one query term"));
    }

    #[test]
    fn shorthand_query_searches_verbatim_words() {
        let args =
            parse_search(&["hello", "packages"]).unwrap();
        assert_eq!(args.query, ["hello", "packages"]);
        assert!(args.mode.is_none());
    }
}
