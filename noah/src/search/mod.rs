pub mod args;
mod backend;
mod query;
mod render;
mod types;

use std::time::Duration;

use crate::search::{
    args::{SearchArgs, SearchKind, SearchMode},
    backend::BackendConfig,
    types::{OptionSearchResult, PackageSearchResult},
};
use color_eyre::{Result, eyre::bail};
use serde::Serialize;
use tracing::{debug, trace};

const CHANNEL: &str = "nixos-unstable";

impl SearchArgs {
    /// Execute the search subcommand.
    ///
    /// # Errors
    ///
    /// Returns an error if no query is provided, if package-only flags are
    /// passed to option search, or if the search request fails.
    pub fn run(&self) -> Result<()> {
        trace!("args: {self:?}");

        let (kind, query) = match &self.mode {
            Some(SearchMode::Packages(args)) => {
                (SearchKind::Packages, args.query.as_slice())
            }
            Some(SearchMode::Options(args)) => {
                (SearchKind::Options, args.query.as_slice())
            }
            None => (self.default_search, self.query.as_slice()),
        };

        if query.is_empty() {
            bail!(
                "no query provided; try `nh search packages <query>`, `nh search \
                 options <query>`, or `nh search --help`"
            );
        }
        if matches!(kind, SearchKind::Options) && self.platforms {
            bail!("--platforms only applies to package search");
        }
        let backend = BackendConfig {
            version: self.backend_version,
            fallbacks: self.backend_fallbacks,
        };

        match kind {
            SearchKind::Packages => run_packages(
                self.limit,
                self.platforms,
                self.json,
                backend,
                query,
            ),
            SearchKind::Options => {
                run_options(self.limit, self.json, backend, query)
            }
        }
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
