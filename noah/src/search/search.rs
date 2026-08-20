use color_eyre::Result;
use tracing::trace;

use crate::search::{args, backend::BackendConfig, online};

impl args::SearchArgs {
    /// Execute the search subcommand.
    ///
    /// # Errors
    ///
    /// Returns an error if no query is provided when using the shorthand form,
    /// if the channel is unsupported, or if the underlying search request fails.
    pub fn run(&self) -> Result<()> {
        trace!("args: {self:?}");
        match self.resolved_mode()? {
            args::ResolvedSearchMode::Packages {
                channel,
                limit,
                platforms,
                backend,
                query,
            } => online::run_packages(
                channel,
                limit,
                platforms,
                self.json,
                BackendConfig {
                    version: backend.version,
                    fallbacks: backend.fallbacks,
                },
                query,
            ),
            args::ResolvedSearchMode::Options {
                channel,
                limit,
                backend,
                query,
            } => online::run_options(
                channel,
                limit,
                self.json,
                BackendConfig {
                    version: backend.version,
                    fallbacks: backend.fallbacks,
                },
                query,
            ),
        }
    }
}
