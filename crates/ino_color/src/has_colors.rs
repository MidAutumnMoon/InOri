//! Check whether ANSI color should be enabled.
//!
//! This implements <https://bixense.com/clicolors>.

use std::io::*;
use std::sync::LazyLock;

pub trait HasColors: IsTerminal {
    fn has_colors( &self ) -> bool;
}

struct EnvSet {
    no_color: bool,
    clicolor_force: bool,
    clicolor: bool,
}

const ENV_SET: LazyLock<EnvSet> = LazyLock::new( || {
    macro_rules! ck {
        ( $n:literal ) => { std::env::var_os( $n ).is_some() }
    }
    EnvSet {
        no_color: ck!( "NO_COLOR" ),
        clicolor_force: ck!( "CLICOLOR_FORCE" ),
        clicolor: ck!( "CLICOLOR" ),
    }
} );

macro_rules! impl_has_color {
    // $target : type, repeated
    // $(,)? : allow trailling comma
    ( $( $target:ty ),* $(,)? ) => { $(
        impl HasColors for $target {
            fn has_colors( &self ) -> bool {
                // NO_COLOR set, don't output any color.
                if ENV_SET.no_color {
                    return false
                }
                // CLICOLOR_FORCE set, output color anyway.
                if ENV_SET.clicolor_force {
                    return true
                }
                // CLICOLOR set, output color only if it's terminal
                if ENV_SET.clicolor {
                    return self.is_terminal()
                }
                // No related envvar set, output color if it's terminal
                return self.is_terminal()
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
