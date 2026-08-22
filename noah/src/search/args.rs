use clap::{Args, Subcommand, ValueEnum};

const DEFAULT_LIMIT: u64 = 30;
const DEFAULT_BACKEND_FALLBACKS: u32 = 1;

#[derive(Args, Debug)]
/// Searches packages or NixOS options via search.nixos.org.
pub struct Search {
    /// Number of search results to display.
    #[arg(
        long,
        short = 'l',
        default_value_t = DEFAULT_LIMIT,
        global = true
    )]
    pub limit: u64,

    /// Show supported platforms for each package.
    #[arg(
        long,
        short = 'P',
        env = "NH_SEARCH_PLATFORM",
        value_parser = clap::builder::BoolishValueParser::new(),
        global = true
    )]
    pub platforms: bool,

    /// Backend index version to query on search.nixos.org. Defaults to the
    /// version bundled with nh.
    #[arg(
        id = "backend-version",
        long = "backend-version",
        env = "NH_SEARCH_BACKEND_VERSION",
        value_name = "VERSION",
        global = true
    )]
    pub backend_version: Option<u32>,

    /// Number of newer index versions to try when the requested version is
    /// outdated (missing on the backend).
    #[arg(
        id = "backend-version-fallbacks",
        long = "backend-version-fallbacks",
        env = "NH_SEARCH_BACKEND_FALLBACKS",
        default_value_t = DEFAULT_BACKEND_FALLBACKS,
        value_name = "COUNT",
        global = true
    )]
    pub backend_fallbacks: u32,

    /// Output results as JSON.
    #[arg(
        long,
        short = 'j',
        env = "NH_SEARCH_JSON",
        value_parser = clap::builder::BoolishValueParser::new(),
        global = true
    )]
    pub json: bool,

    /// Default search mode used when no subcommand is given.
    /// Accepts `packages` or `options`.
    #[arg(
        long,
        env = "NH_DEFAULT_SEARCH",
        default_value = "packages",
        value_name = "MODE"
    )]
    pub default_search: SearchKind,

    #[command(subcommand)]
    pub mode: Option<SearchMode>,

    /// Query shorthand: equivalent to `nh search packages <query>` or
    /// `nh search options <query>` depending on `--default-search`.
    pub query: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum SearchMode {
    /// Search packages via search.nixos.org.
    Packages(Packages),
    /// Search NixOS options via search.nixos.org.
    Options(Options),
}

#[derive(Args, Debug)]
pub struct Packages {
    /// Name of the package to search.
    #[arg(required = true)]
    pub query: Vec<String>,
}

#[derive(Args, Debug)]
pub struct Options {
    /// Name of the option to search.
    #[arg(required = true)]
    pub query: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum SearchKind {
    /// Search packages (default).
    #[default]
    Packages,
    /// Search NixOS options.
    Options,
}

#[cfg(test)]
mod tests {
    use clap::{Parser, Subcommand, error::ErrorKind};
    use std::assert_matches;

    use super::{Search, SearchKind, SearchMode};

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: TestCommand,
    }

    #[derive(Debug, Subcommand)]
    enum TestCommand {
        Search(Search),
    }

    fn parse_search(args: &[&str]) -> clap::error::Result<Search> {
        let cli = TestCli::try_parse_from(
            std::iter::once("nh").chain(args.iter().copied()),
        )?;
        match cli.command {
            TestCommand::Search(search) => Ok(search),
        }
    }

    #[test]
    fn global_flags_work_on_both_sides_of_subcommand()
    -> clap::error::Result<()> {
        let args = parse_search(&[
            "search",
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
        ])?;

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
                return Err(clap::Error::raw(
                    ErrorKind::InvalidValue,
                    format!("expected packages mode, got {other:?}"),
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn shorthand_flags_parse_after_query() -> clap::error::Result<()> {
        let args = parse_search(&[
            "search",
            "hello",
            "--limit",
            "5",
            "--platforms",
            "--default-search",
            "packages",
        ])?;

        assert_eq!(args.limit, 5);
        assert!(args.platforms);
        assert_matches!(args.default_search, SearchKind::Packages);
        assert_eq!(args.query, ["hello"]);
        assert!(args.mode.is_none());
        Ok(())
    }

    #[test]
    fn default_search_parses_after_shorthand_query()
    -> clap::error::Result<()> {
        let args = parse_search(&[
            "search",
            "hello",
            "--default-search",
            "options",
        ])?;

        assert_matches!(args.default_search, SearchKind::Options);
        assert_eq!(args.query, ["hello"]);
        assert!(args.mode.is_none());
        Ok(())
    }

    #[test]
    fn backend_flags_have_shared_defaults() -> clap::error::Result<()> {
        let args = parse_search(&["search", "options", "hello"])?;
        assert_eq!(args.backend_version, None);
        assert_eq!(args.backend_fallbacks, 1);
        Ok(())
    }

    #[test]
    fn options_preserve_platform_flag_for_runtime_validation()
    -> clap::error::Result<()> {
        let args =
            parse_search(&["search", "options", "hello", "--platforms"])?;
        assert!(args.platforms);
        assert_matches!(args.mode, Some(SearchMode::Options(_)));
        Ok(())
    }

    #[test]
    fn subcommand_requires_query() -> clap::error::Result<()> {
        match parse_search(&["search", "packages"]) {
            Ok(args) => Err(clap::Error::raw(
                ErrorKind::InvalidValue,
                format!("expected missing-query error, got {args:?}"),
            )),
            Err(error) => {
                assert_eq!(
                    error.kind(),
                    ErrorKind::MissingRequiredArgument
                );
                Ok(())
            }
        }
    }
}
