mod backend;
mod cli;
mod query;
mod render;
mod types;
pub use cli::search_cli;

use std::time::Duration;

use crate::search::{
    backend::BackendConfig,
    types::{OptionSearchResult, PackageSearchResult},
};
use rootcause::Result;
use serde::Serialize;
use tracing::{debug, trace};

const CHANNEL: &str = "nixos-unstable";

#[derive(Clone, Debug)]
pub struct Request {
    pub target: Target,
    pub query: Vec<String>,
    pub limit: u64,
    pub backend: BackendConfig,
    pub json: bool,
}

#[derive(Clone, Debug)]
pub enum Target {
    Packages { platforms: bool },
    Options,
}

/// Execute a canonical search request.
///
/// # Errors
///
/// Returns an error if the search request fails.
pub fn run(request: &Request) -> Result<()> {
    trace!(?request);

    match request.target {
        Target::Packages { platforms } => run_packages(
            request.limit,
            platforms,
            request.json,
            request.backend,
            &request.query,
        ),
        Target::Options => run_options(
            request.limit,
            request.json,
            request.backend,
            &request.query,
        ),
    }
}

fn run_packages(
    limit: u64,
    platforms: bool,
    json: bool,
    backend: BackendConfig,
    query: &[String],
) -> Result<()> {
    let query_s = query.join(" ");
    debug!(?query_s);

    let search = query::packages(&query_s, limit);
    if !json {
        println!("Querying search.nixos.org, with channel {CHANNEL}...");
    }

    let (documents, elapsed) = backend::search_documents::<
        PackageSearchResult,
    >(&search, CHANNEL, backend)?;

    finish(json, &query_s, elapsed, &documents, |documents| {
        render::print_packages(platforms, documents);
    })
}

fn run_options(
    limit: u64,
    json: bool,
    backend: BackendConfig,
    query: &[String],
) -> Result<()> {
    let query_s = query.join(" ");
    debug!(?query_s);

    let search = query::options(&query_s, limit);
    if !json {
        println!(
            "Querying options on search.nixos.org, with channel {CHANNEL}..."
        );
    }

    let (documents, elapsed) = backend::search_documents::<
        OptionSearchResult,
    >(&search, CHANNEL, backend)?;

    finish(json, &query_s, elapsed, &documents, render::print_options)
}

/// Shared output tail: one JSON document, or the rendered result list.
fn finish<T>(
    json: bool,
    query: &str,
    elapsed: Duration,
    documents: &[T],
    render: impl FnOnce(&[T]),
) -> Result<()>
where
    T: Serialize,
{
    if json {
        let output = types::JsonOutput {
            query,
            channel: CHANNEL,
            elapsed_ms: elapsed.as_millis(),
            results: documents,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("Took {}ms", elapsed.as_millis());
    println!("Most relevant results at the end");
    println!();
    render(documents);
    Ok(())
}
