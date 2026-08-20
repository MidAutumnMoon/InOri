use std::time::{Duration, Instant};

use color_eyre::{
    Result,
    eyre::{Context, ContextCompat, bail, eyre},
};
use elasticsearch_dsl::{Search, SearchResponse};
use serde::de::DeserializeOwned;
use tracing::{debug, trace, warn};

const NH_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Backend index version bundled with nh, used when the user does not override
/// it via [`BackendConfig::version`].
pub const BUNDLED_BACKEND_VERSION: &str =
    include_str!("BACKEND_VERSION");

/// Backend index version selection for a search request.
#[derive(Clone, Copy)]
pub struct BackendConfig {
    /// Index version to try first. `None` uses [`BUNDLED_BACKEND_VERSION`].
    pub version: Option<u32>,
    /// Number of newer versions to try when the requested one is outdated.
    pub fallbacks: u32,
}

/// Outcome of a single request to a specific backend index version.
enum BackendResponse {
    Found(String),
    /// The index does not exist, so the requested version is outdated.
    Outdated,
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
    let body = loop {
        match query_backend(query, channel, version)? {
            BackendResponse::Found(body) => break body,
            BackendResponse::Outdated => {
                if version >= last {
                    if start == last {
                        bail!(
                            "search.nixos.org has no index for channel '{channel}' at \
               backend version {start}. The channel may not exist, or the \
               version may be wrong."
                        );
                    }
                    bail!(
                        "search.nixos.org has no index for channel '{channel}' at backend \
             versions {start} through {last}. The channel may not exist, or \
             nh may be too old to query it."
                    );
                }
                let next = version + 1;
                warn!(
                    "Backend index version {version} is outdated, retrying with {next}. \
           Consider updating nh."
                );
                version = next;
            }
        }
    };

    let elapsed = then.elapsed();
    debug!(?elapsed);

    let parsed_response: SearchResponse = serde_json::from_str(&body)
        .context("parsing response into the elasticsearch format")?;
    trace!(?parsed_response);

    let documents =
        parsed_response.documents::<T>().context("parsing the search documents")?;
    Ok((documents, elapsed))
}

/// Queries a single backend index version.
///
/// Returns [`BackendResponse::Outdated`] on a 404 (missing index) so the caller
/// can retry a newer version. Any other non-success status is a hard error.
fn query_backend(
    query: &Search,
    channel: &str,
    version: u32,
) -> Result<BackendResponse> {
    let body = serde_json::to_string(query).context("building the search query")?;
    let url = format!(
        "https://search.nixos.org/backend/latest-{version}-{channel}/_search"
    );

    debug!(url, body);

    // The HTTP status code is appended to stdout after the response body.
    let output = std::process::Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--request")
        .arg("POST")
        .arg("--header")
        .arg("Content-Type: application/json")
        .arg("--user-agent")
        .arg(format!("nh/{NH_VERSION}"))
        // Hardcoded upstream
        // https://github.com/NixOS/nixos-search/blob/744ec58e082a3fcdd741b2c9b0654a0f7fda4603/frontend/src/index.js
        .arg("--user")
        .arg("aWVSALXpZv:X8gPHnzL52wFEekuxsfQ9cSh")
        .arg("--data")
        .arg(&body)
        .arg("--write-out")
        .arg("\n%{http_code}")
        .arg(&url)
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                eyre!("`curl` was not found in PATH, but is required for searching")
            } else {
                eyre!(err).wrap_err("querying the elasticsearch API")
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "querying the elasticsearch API: curl exited with {}: {stderr}",
            output.status
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let (body, status) = stdout
        .rsplit_once('\n')
        .context("splitting curl's status code from the response body")?;
    let status: u16 = status.parse().context("parsing curl's status code")?;

    trace!(status, body);

    if status == 404 {
        return Ok(BackendResponse::Outdated);
    }

    if !(200..300).contains(&status) {
        eprintln!(
            "Error: search.nixos.org returned HTTP {status} for channel '{channel}'. This \
       usually means the channel does not exist, is not indexed, or the \
       request was malformed.",
        );
        bail!(
            "search.nixos.org returned HTTP {status} for channel '{channel}'",
        );
    }

    Ok(BackendResponse::Found(body.into()))
}
