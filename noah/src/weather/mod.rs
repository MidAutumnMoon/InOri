use bpaf::Parser;
use bpaf::construct;
use bpaf::long;
use rootcause::Result;
use tracing::instrument;

use crate::runtime::Config;
use crate::runtime::Env;

#[derive(Clone, Debug)]
pub struct WeatherRequest {
    /// Hostname to get forecast for.
    pub hostname: String,
}

/// Execute a weather request.
///
/// # Errors
///
/// Returns an error if the weather query or reporting fails.
#[instrument(skip_all)]
pub fn run(_request: &WeatherRequest, config: &Config) -> Result<()> {
    dbg!(&config);
    todo!()
}

/// Assemble the `nh weather` parser.
#[must_use]
pub fn weather_cli() -> impl Parser<WeatherRequest> {
    // TODO: Pass env or config to *_cli() and get rid of unwrap.
    #[expect(
        clippy::expect_used,
        reason = "Remove this in later refactor"
    )]
    let hostname = Env::hostname().expect("hostname");

    let hostname = long("hostname")
        .short('H')
        .help("Hostname to weather")
        .argument("HOSTNAME")
        .fallback(hostname);

    construct!(WeatherRequest { hostname })
}

fn nix_build_dry_run_parser() {}
