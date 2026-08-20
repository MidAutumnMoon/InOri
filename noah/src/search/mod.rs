pub mod args;
mod backend;
mod query;
mod render;
mod types;

use std::time::Duration;

use crate::search::{
    args::{BackendArgs, SearchArgs, SearchDefault, SearchMode},
    backend::BackendConfig,
    types::{OptionSearchResult, PackageSearchResult},
};
use color_eyre::{Result, eyre::bail};
use serde::Serialize;
use tracing::{debug, trace};

impl SearchArgs {
    /// Execute the search subcommand.
    ///
    /// # Errors
    ///
    /// Returns an error if no query is provided when using the shorthand form,
    /// if package-only flags are passed to option search, or if the underlying
    /// search request fails.
    pub fn run(&self) -> Result<()> {
        trace!("args: {self:?}");

        match &self.mode {
            Some(SearchMode::Packages(args)) => run_packages(
                &args.channel.value,
                args.limit.value,
                args.platforms.value,
                self.json,
                args.backend,
                &args.query,
            ),
            Some(SearchMode::Options(args)) => run_options(
                &args.channel.value,
                args.limit.value,
                self.json,
                args.backend,
                &args.query,
            ),
            None => {
                if self.query.is_empty() {
                    bail!(
                        "no query provided; try `nh search packages <query>`, `nh search \
             options <query>`, or `nh search --help`"
                    );
                }

                match self.default_search {
                    SearchDefault::Packages => run_packages(
                        &self.channel.value,
                        self.limit.value,
                        self.platforms.value,
                        self.json,
                        self.backend,
                        &self.query,
                    ),
                    SearchDefault::Options => {
                        if self.platforms.value {
                            bail!(
                                "--platforms only applies to package search"
                            );
                        }

                        run_options(
                            &self.channel.value,
                            self.limit.value,
                            self.json,
                            self.backend,
                            &self.query,
                        )
                    }
                }
            }
        }
    }
}

fn run_packages(
    channel: &str,
    limit: u64,
    platforms: bool,
    json: bool,
    backend: BackendArgs,
    query: &[String],
) -> Result<()> {
    let query_s = query.join(" ");
    debug!(?query_s);

    let search = query::packages(&query_s, limit);

    if !json {
        println!("Querying search.nixos.org, with channel {channel}...");
    }

    let (documents, elapsed) = backend::search_documents::<
        PackageSearchResult,
    >(&search, channel, backend.into())?;

    finish(
        json,
        &query_s,
        channel,
        elapsed,
        &documents,
        |channel, documents| {
            render::packages::print(channel, platforms, documents);
        },
    )
}

fn run_options(
    channel: &str,
    limit: u64,
    json: bool,
    backend: BackendArgs,
    query: &[String],
) -> Result<()> {
    let query_s = query.join(" ");
    debug!(?query_s);

    let search = query::options(&query_s, limit);

    if !json {
        println!(
            "Querying options on search.nixos.org, with channel {channel}..."
        );
    }

    let (documents, elapsed) = backend::search_documents::<
        OptionSearchResult,
    >(&search, channel, backend.into())?;

    finish(
        json,
        &query_s,
        channel,
        elapsed,
        &documents,
        render::options::print,
    )
}

/// Shared output tail: one JSON document, or the rendered result list.
fn finish<T>(
    json: bool,
    query: &str,
    channel: &str,
    elapsed: Duration,
    documents: &[T],
    render: impl FnOnce(&str, &[T]),
) -> Result<()>
where
    T: Serialize,
{
    if json {
        let output = types::JsonOutput {
            query,
            channel,
            elapsed_ms: elapsed.as_millis(),
            results: documents,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    println!("Took {}ms", elapsed.as_millis());
    println!("Most relevant results at the end");
    println!();
    render(channel, documents);
    Ok(())
}

impl From<BackendArgs> for BackendConfig {
    fn from(args: BackendArgs) -> Self {
        Self {
            version: args.version,
            fallbacks: args.fallbacks,
        }
    }
}
