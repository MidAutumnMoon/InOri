// #[expect] has subtle bugs with wildcard_imports, so use allow instead :/
#![allow(clippy::wildcard_imports, reason = "prelude pattern")]

use std::env;
use std::io::IsTerminal as _;
use std::io::stderr;

use tracing_appender::non_blocking::NonBlockingBuilder;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::*;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry;

/// Init custom [`tracing_subscriber`] configuration.
#[expect(clippy::inline_always, reason = "It eliminates crate boundary")]
#[inline(always)]
#[must_use = "Must keep the log guard alive (Don't use `let _` !)"]
pub fn init_tracing_subscriber() -> WorkerGuard {
    // Explicity acknowledge that non-blocking is lossy
    let (nb_writer, log_guard) =
        NonBlockingBuilder::default().lossy(true).finish(stderr());

    let fmt_layer = fmt::layer()
        .with_writer(nb_writer)
        .with_file(false)
        .with_line_number(false)
        .without_time()
        .with_ansi(stderr().is_terminal());

    let env_filter = env::var("RUST_LOG")
        .ok()
        .and_then(|value| value.parse::<Targets>().ok())
        .unwrap_or_else(|| Targets::new().with_default(LevelFilter::INFO));

    registry().with(fmt_layer).with(env_filter).init();

    log_guard
}
