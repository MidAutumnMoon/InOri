use std::time::{Duration, Instant};

use elasticsearch_dsl::{Search, SearchResponse};
use rootcause::{
    Result, bail, option_ext::OptionExt as _, prelude::ResultExt as _, report,
};
use serde::de::DeserializeOwned;
use subprocess::Exec;
use tracing::{debug, trace, warn};

const NH_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Backend index version bundled with nh, used when the user does not override
/// it via [`BackendConfig::version`].
pub const BUNDLED_BACKEND_VERSION: &str = include_str!("BACKEND_VERSION");

/// Backend index version selection for a search request.
#[derive(Clone, Copy)]
pub struct BackendConfig {
    /// Index version to try first. `None` uses [`BUNDLED_BACKEND_VERSION`].
    pub version: Option<u32>,
    /// Number of newer versions to try when the requested one is outdated.
    pub fallbacks: u32,
}

pub fn search_documents<T>(
    query: &Search,
    channel: &str,
    config: BackendConfig,
) -> Result<(Vec<T>, Duration)>
where
    T: DeserializeOwned,
{
    let start = match config.version {
        Some(version) => version,
        None => BUNDLED_BACKEND_VERSION
            .trim()
            .parse()
            .context("parsing the bundled backend index version")?,
    };
    let last = start.saturating_add(config.fallbacks);

    let then = Instant::now();

    // The requested index version tracks search.nixos.org but can fall behind
    // between releases. A missing index answers with 404, so when a version is
    // outdated we retry against successively newer versions, up to `fallbacks`
    // times, before giving up.
    let mut version = start;
    let response = loop {
        if let Some(response) = query_backend(query, channel, version)? {
            break response;
        }
        if version >= last {
            if start == last {
                bail!(
                    "search.nixos.org has no index for channel '{channel}' at \
                     backend version {start}. The version may be wrong."
                );
            }
            bail!(
                "search.nixos.org has no index for channel '{channel}' at backend \
                 versions {start} through {last}. nh may be too old to query it."
            );
        }
        let next = version + 1;
        warn!(
            "Backend index version {version} is outdated, retrying with {next}. \
             Consider updating nh."
        );
        version = next;
    };

    let elapsed = then.elapsed();
    debug!(?elapsed);
    trace!(?response);

    let documents = response
        .documents::<T>()
        .context("parsing the search documents")?;
    Ok((documents, elapsed))
}

/// Queries a single backend index version.
///
/// Returns `None` on a 404 (missing index) so the caller can retry a newer
/// version. Any other non-success status is a hard error.
fn query_backend(
    query: &Search,
    channel: &str,
    version: u32,
) -> Result<Option<SearchResponse>> {
    let body = serde_json::to_string(query)
        .context("building the search query")?;
    let url = format!(
        "https://search.nixos.org/backend/latest-{version}-{channel}/_search"
    );

    debug!(url, body);

    // Feed JSON through stdin so query length is not constrained by argv. Curl
    // appends the HTTP status code to stdout after the response body.
    let output = Exec::cmd("curl")
        .args([
            "--silent",
            "--show-error",
            "--request",
            "POST",
            "--header",
            "Content-Type: application/json",
            "--user-agent",
        ])
        .arg(format!("nh/{NH_VERSION}"))
        // Hardcoded upstream
        // https://github.com/NixOS/nixos-search/blob/744ec58e082a3fcdd741b2c9b0654a0f7fda4603/frontend/src/index.js
        .args([
            "--user",
            "aWVSALXpZv:X8gPHnzL52wFEekuxsfQ9cSh",
            "--data-binary",
            "@-",
            "--write-out",
            "\n%{http_code}",
        ])
        .arg(&url)
        .stdin(body)
        .capture()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                report!("`curl` was not found in PATH, but is required for searching")
            } else {
                report!(error)
                    .context("querying the elasticsearch API")
                    .into()
            }
        })?;

    if !output.success() {
        let stderr = output.stderr_str();
        bail!(
            "querying the elasticsearch API: curl exited with {}: {}",
            output.exit_status,
            stderr.trim()
        );
    }

    let separator = output
        .stdout
        .iter()
        .rposition(|byte| *byte == b'\n')
        .context("splitting curl's status code from the response body")?;
    let (body, status) = output.stdout.split_at(separator);
    let status = status.get(1..).context("reading curl's status code")?;
    let status = std::str::from_utf8(status)
        .context("reading curl's status code")?
        .parse::<u16>()
        .context("parsing curl's status code")?;

    trace!(status);

    if status == 404 {
        return Ok(None);
    }
    if !(200..300).contains(&status) {
        bail!(
            "search.nixos.org returned HTTP {status} for channel '{channel}'. The backend \
             request was rejected or malformed."
        );
    }

    let response = serde_json::from_slice(body)
        .context("parsing response into the elasticsearch format")?;
    Ok(Some(response))
}
