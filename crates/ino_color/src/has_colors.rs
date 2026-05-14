//! Check whether ANSI color should be enabled.
//!
//! This implements <https://bixense.com/clicolors>.

#[allow(clippy::wildcard_imports)]
use std::io::*;
use std::sync::LazyLock;

pub trait HasColors: IsTerminal {
    fn has_colors(&self) -> bool;
}

struct EnvSet {
    no_color: bool,
    clicolor_force: bool,
    /// `None` = not set, `Some(true)` = `1`, `Some(false)` = `0`.
    clicolor: Option<bool>,
}

static ENV_SET: LazyLock<EnvSet> = LazyLock::new(|| {
    EnvSet {
        // NO_COLOR is presence-based per its own spec.
        no_color: std::env::var_os("NO_COLOR").is_some(),
        clicolor_force: std::env::var("CLICOLOR_FORCE")
            .is_ok_and(|v| v != "0"),
        clicolor: std::env::var("CLICOLOR").ok().and_then(|v| {
            match v.as_str() {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            }
        }),
    }
});

macro_rules! impl_has_color {
    // $target : type, repeated
    // $(,)? : allow trailling comma
    ( $( $target:ty ),* $(,)? ) => { $(
        impl HasColors for $target {
            #[ inline ]
            fn has_colors( &self ) -> bool {
                // NO_COLOR set, don't output any color.
                if ENV_SET.no_color {
                    return false
                }
                // CLICOLOR_FORCE set (non-zero), output color anyway.
                if ENV_SET.clicolor_force {
                    return true
                }
                // CLICOLOR=0 disables; CLICOLOR=1 or unset defaults to tty.
                match ENV_SET.clicolor {
                    Some(true) => self.is_terminal(),
                    Some(false) => false,
                    None => self.is_terminal(),
                }
            }
        }
    )* }
}

impl_has_color! {
    std::fs::File,
    std::os::fd::OwnedFd,
    std::os::fd::BorrowedFd<'_>,
    Stdin, StdinLock<'_>,
    Stdout, StdoutLock<'_>,
    Stderr, StderrLock<'_>,
}
