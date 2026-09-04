use std::str::FromStr;

use bpaf::{Parser, construct, long, positional};

use super::Request;
use super::Target;
use super::backend::BackendConfig;

const DEFAULT_LIMIT: u64 = 30;
const DEFAULT_BACKEND_FALLBACKS: u32 = 1;

#[derive(Debug, Clone, Copy, Default)]
enum SearchKind {
    #[default]
    Packages,
    Options,
}

impl FromStr for SearchKind {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
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

/// Parser-only flags shared by explicit modes and shorthand search.
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
    let platforms = long("platforms")
        .short('P')
        .help("Show supported platforms for each package")
        .switch();
    let backend_version = long("backend-version")
        .argument::<u32>("VERSION")
        .help(
            "Backend index version to query on search.nixos.org. Defaults \
             to the version bundled with nh",
        )
        .optional();
    let backend_fallbacks = long("backend-version-fallbacks")
        .argument::<u32>("COUNT")
        .help(
            "Number of newer index versions to try when the requested \
             version is outdated (missing on the backend)",
        )
        .fallback(DEFAULT_BACKEND_FALLBACKS)
        .display_fallback();
    let json = long("json")
        .short('j')
        .help("Output results as JSON")
        .switch();
    let default_search = long("default-search")
        .argument::<SearchKind>("MODE")
        .help(
            "Default search mode used when no subcommand is given. Accepts \
             `packages` or `options`",
        )
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

#[derive(Clone, Debug)]
enum ExplicitMode {
    Packages(Vec<String>),
    Options(Vec<String>),
}

#[must_use]
fn packages_cli() -> impl Parser<ExplicitMode> {
    positional::<String>("QUERY")
        .help("Name of the package to search")
        .many()
        .to_options()
        .descr("Search packages via search.nixos.org.")
        .command("packages")
        .map(ExplicitMode::Packages)
}

#[must_use]
fn options_cli() -> impl Parser<ExplicitMode> {
    positional::<String>("QUERY")
        .help("Name of the option to search")
        .many()
        .to_options()
        .descr("Search NixOS options via search.nixos.org.")
        .command("options")
        .map(ExplicitMode::Options)
}

#[derive(Clone, Debug)]
struct SearchWithMode {
    flags: SearchFlags,
    mode: ExplicitMode,
}

#[derive(Clone, Debug)]
struct SearchWithoutMode {
    flags: SearchFlags,
    query: Vec<String>,
}

#[derive(Clone, Debug)]
enum RawSearch {
    Explicit(SearchWithMode),
    Shorthand(SearchWithoutMode),
}

fn normalize(raw: RawSearch) -> std::result::Result<Request, String> {
    let (flags, kind, query) = match raw {
        RawSearch::Explicit(SearchWithMode { flags, mode }) => {
            match mode {
                ExplicitMode::Packages(query) => {
                    (flags, SearchKind::Packages, query)
                }
                ExplicitMode::Options(query) => {
                    (flags, SearchKind::Options, query)
                }
            }
        }
        RawSearch::Shorthand(SearchWithoutMode { flags, query }) => {
            let kind = flags.default_search;
            (flags, kind, query)
        }
    };

    if query.is_empty() {
        return Err(String::from(
            "expected at least one search query term",
        ));
    }

    let target = match kind {
        SearchKind::Packages => Target::Packages {
            platforms: flags.platforms,
        },
        SearchKind::Options if flags.platforms => {
            return Err(String::from(
                "`--platforms` only applies to package search",
            ));
        }
        SearchKind::Options => Target::Options,
    };

    Ok(Request {
        target,
        query,
        limit: flags.limit,
        backend: BackendConfig {
            version: flags.backend_version,
            fallbacks: flags.backend_fallbacks,
        },
        json: flags.json,
    })
}

/// Parse explicit `packages`/`options` subcommands or shorthand search into one
/// canonical request.
pub fn search_cli() -> impl Parser<Request> {
    let with_mode = {
        let flags = search_flags();
        let mode = construct!([packages_cli(), options_cli()]);
        construct!(SearchWithMode { flags, mode }).map(RawSearch::Explicit)
    };
    let without_mode = {
        let flags = search_flags();
        let query = positional::<String>("QUERY")
            .help(
                "Query shorthand: equivalent to `nh search packages <query>` \
                 or `nh search options <query>` depending on --default-search",
            )
            .many();
        construct!(SearchWithoutMode { flags, query })
            .map(RawSearch::Shorthand)
    };

    construct!([with_mode, without_mode]).parse(normalize)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Test assertions")]
mod tests {
    use bpaf::{Args, ParseFailure, Parser as _};

    use super::Request;
    use super::Target;
    use super::search_cli;

    fn parse_search(
        args: &[&str],
    ) -> std::result::Result<Request, String> {
        let options = search_cli().to_options();
        options.check_invariants(false);
        options
            .run_inner(Args::from(args).set_name("search"))
            .map_err(ParseFailure::unwrap_stderr)
    }

    #[test]
    fn global_flags_work_on_both_sides_of_subcommand() {
        let request = parse_search(&[
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

        assert_eq!(request.limit, 5);
        assert!(request.json);
        assert_eq!(request.backend.version, Some(51));
        assert_eq!(request.backend.fallbacks, 3);
        assert_eq!(request.query, ["hello"]);
        assert!(matches!(
            request.target,
            Target::Packages { platforms: true }
        ));
    }

    #[test]
    fn shorthand_flags_parse_after_query() {
        let request = parse_search(&[
            "hello",
            "--limit",
            "5",
            "--platforms",
            "--default-search",
            "packages",
        ])
        .unwrap();

        assert_eq!(request.limit, 5);
        assert_eq!(request.query, ["hello"]);
        assert!(matches!(
            request.target,
            Target::Packages { platforms: true }
        ));
    }

    #[test]
    fn default_search_selects_shorthand_target() {
        let request =
            parse_search(&["hello", "--default-search", "options"])
                .unwrap();

        assert_eq!(request.query, ["hello"]);
        assert!(matches!(request.target, Target::Options));
    }

    #[test]
    fn backend_flags_have_shared_defaults() {
        let request = parse_search(&["options", "hello"]).unwrap();
        assert_eq!(request.backend.version, None);
        assert_eq!(request.backend.fallbacks, 1);
    }

    #[test]
    fn options_reject_platform_flag() {
        let error = parse_search(&["options", "hello", "--platforms"])
            .unwrap_err();
        assert!(error.contains("only applies to package search"));
    }

    #[test]
    fn search_requires_query() {
        for args in [&["packages"][..], &[][..]] {
            let error = parse_search(args).unwrap_err();
            assert!(
                error.contains("expected at least one search query term")
            );
        }
    }

    #[test]
    fn shorthand_query_searches_verbatim_words() {
        let request = parse_search(&["hello", "packages"]).unwrap();
        assert_eq!(request.query, ["hello", "packages"]);
        assert!(matches!(
            request.target,
            Target::Packages { platforms: false }
        ));
    }
}
