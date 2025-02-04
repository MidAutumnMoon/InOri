//! Check whether ANSI color should be enabled.
//!
//! This implements <https://bixense.com/clicolors>.

use std::io::*;

pub trait HasColors: IsTerminal {
    fn has_colors( &self ) -> bool;
}

macro_rules! impl_has_color {
    // $target : type, repeated
    // $(,)? : allow trailling comma
    ( $( $target:ty ),* $(,)? ) => { $(
        impl HasColors for $target {
            fn has_colors( &self ) -> bool {
                #[ inline ]
                fn var_set( name: &str ) -> bool {
                    std::env::var_os( name ).is_some()
                }
                let no_color = var_set( "NO_COLOR" );
                let clicolor_force = var_set( "CLICOLOR_FORCE" );
                let clicolor = var_set( "CLICOLOR" );
                // NO_COLOR set, don't output any color.
                if no_color {
                    return false
                }
                // CLICOLOR_FORCE set, output color anyway.
                if clicolor_force {
                    return true
                }
                // CLICOLOR set, output color only if it's terminal
                if clicolor {
                    return self.is_terminal()
                }
                // No related envvar set, output color if it's terminal
                return self.is_terminal()
            }
        }
    )* }
}

impl_has_color!(
    std::fs::File,
    std::os::fd::OwnedFd,
    std::os::fd::BorrowedFd<'_>,
    Stdin, StdinLock<'_>,
    Stdout, StdoutLock<'_>,
    Stderr, StderrLock<'_>,
);
