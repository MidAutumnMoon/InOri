//! Coloring the terminal output.
//!
//! # Basic Usage
//!
//! ```rust
//! use ino_color::cprintln;
//! use ino_color::fg;
//! use ino_color::bg;
//! use ino_color::style;
//!
//! // Foreground color only
//! cprintln!(fg::Yellow, "Hello Fancy");
//!
//! // Foreground + style
//! cprintln!((fg::Blue, style::Italic), "Savoy blue");
//!
//! // Foreground + background + style
//! cprintln!((fg::Green, bg::Black, style::Bold),
//!     "Green on black, bold");
//!
//! // All format traits work as expected
//! cprintln!(fg::Green, "{:?}", vec![123]);
//! cprintln!(fg::Green, "{:X}", 123);
//! ```

use std::io::{
    IsTerminal, Stderr, StderrLock, Stdin, StdinLock, Stdout, StdoutLock,
};
use std::sync::LazyLock;

/// Check whether ANSI color should be enabled.
///
/// This implements <https://force-color.org/> and
/// <https://bixense.com/clicolors>.
///
/// Environment variables are read **once** at first use and cached.
/// Runtime changes to `NO_COLOR`, `FORCE_COLOR`, `CLICOLOR_FORCE`,
/// or `CLICOLOR` after that point will not be reflected.
pub trait HasColors: IsTerminal {
    fn has_colors(&self) -> bool;
}

struct EnvSet {
    no_color: bool,
    force_color: bool,
    clicolor_force: bool,
    /// `None` = not set, `Some(true)` = `1`, `Some(false)` = `0`.
    clicolor: Option<bool>,
}

static ENV_SET: LazyLock<EnvSet> = LazyLock::new(|| {
    EnvSet {
        // NO_COLOR is presence-based per its own spec.
        no_color: std::env::var_os("NO_COLOR").is_some(),
        // FORCE_COLOR: present and non-empty → force color.
        // Per <https://force-color.org/>.
        force_color: std::env::var_os("FORCE_COLOR")
            .is_some_and(|v| !v.is_empty()),
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
    ( $( $target:ty ),* $(,)? ) => { $(
        impl HasColors for $target {
            #[inline]
            fn has_colors(&self) -> bool {
                // Priority: FORCE_COLOR > NO_COLOR > CLICOLOR_FORCE > CLICOLOR > default (tty).
                // FORCE_COLOR overrides everything per force-color.org.
                if ENV_SET.force_color {
                    return true;
                }
                // NO_COLOR set, don't output any color.
                if ENV_SET.no_color {
                    return false;
                }
                // CLICOLOR_FORCE set (non-zero), output color anyway.
                if ENV_SET.clicolor_force {
                    return true;
                }
                // CLICOLOR=0 disables; CLICOLOR=1 or unset defaults to tty.
                match ENV_SET.clicolor {
                    Some(true) => self.is_terminal(),
                    Some(false) => false,
                    None => self.is_terminal(),
                }
            }
        }
    )* };
}

impl_has_color! {
    std::fs::File,
    std::os::fd::OwnedFd,
    std::os::fd::BorrowedFd<'_>,
    Stdin, StdinLock<'_>,
    Stdout, StdoutLock<'_>,
    Stderr, StderrLock<'_>,
}

/// An attribute in the [ANSI SGR](https://w.wiki/DBZ2) list.
pub trait AnsiSgr {
    const ATTR: &'static str;
}

/// The corresponding attribute is for *foreground color*.
pub trait FG: AnsiSgr {}
/// The corresponding attribute is for *background color*.
pub trait BG: AnsiSgr {}
/// The corresponding attribute is for attributes which mainly
/// affects the *style* of output, such as italic or bold.
pub trait Style: AnsiSgr {}

macro_rules! lets_colors {
    ( $( $name:ident $fg:literal $bg:literal ),* $(,)? ) => {
        /// Named 16 foreground colors.
        pub mod fg { $(
            pub struct $name;
            impl crate::AnsiSgr for $name {
                const ATTR: &'static str = stringify!( $fg );
            }
            impl crate::FG for $name {}
        )* }
        /// Named 16 background colors.
        pub mod bg { $(
            pub struct $name;
            impl crate::AnsiSgr for $name {
                const ATTR: &'static str = stringify!( $bg );
            }
            impl crate::BG for $name {}
        )* }
    }
}
lets_colors! {
    Default   39 49,
    Black   30 40,
    Red     31 41,
    Green   32 42,
    Yellow  33 43,
    Blue    34 44,
    Magenta 35 45,
    Cyan    36 46,
    White   37 47,
    BrightBlack   90 100,
    BrightRed     91 101,
    BrightGreen   92 102,
    BrightYellow  93 103,
    BrightBlue    94 104,
    BrightMagenta 95 105,
    BrightCyan    96 106,
    BrightWhite   97 107,
}

macro_rules! lets_styles {
    ( $( $name:ident $attr:literal ),* $(,)? ) => {
        /// Commonly used style attributes.
        pub mod style { $(
            pub struct $name;
            impl crate::AnsiSgr for $name {
                const ATTR: &'static str = stringify!( $attr );
            }
            impl crate::Style for $name {}
        )* }
    }
}
lets_styles! {
    // Reset (SGR 0) clears all attributes. Using it as a style
    // in a tuple like `(Blue, Reset)` would produce `\e[34;0m`
    // which immediately undoes the color — almost certainly not
    // what the caller wants.
    Reset 0,
    Bold 1,
    Dim 2,
    Italic 3,
    Underline 4,
    Blink 5,
    // Rapid_blink 6,
    Invert 7,
    Hide 8,
    Strike 9,
    DoubleUnderline 21,
    Overline 53,
}

/// Helper: conditionally write a trailing newline.
///
/// Called by the print macros; `true` → `writeln!`,
/// `false` → nothing.
#[macro_export]
#[doc(hidden)]
macro_rules! __ino_newline {
    (true, $lock:ident) => {
        writeln!($lock).unwrap()
    };
    (false, $lock:ident) => {};
}

/// Create the color printing macros.
///
/// ## Dollar sign workaround
///
/// To create new macros, nested macro is used.
/// However, it hits sorta of rustc limitation and the dollar sign
/// needs to be escaped while creating nested macros.
///
/// Ref: <https://github.com/rust-lang/rust/issues/35853>
macro_rules! create_print_macro {
    // Create a single macro
    // `$dol`          the "escaped" dollar sign
    // `$print_macro`  the underlying std print macro (for docs)
    // `$stream`       the stream function for HasColors check
    // `$newline`      whether to append a trailing newline
    (
        $name:ident,
        $print_macro:path,
        $stream:path,
        $newline:tt,
        $dol:tt
    ) => {
        #[macro_export]
        #[doc = concat!(
            "Print with color, wraps [`", stringify!($print_macro), "!`].",
            "\n\n",
            "## Syntax\n",
            "- `", stringify!($name), "!(FG, ..)` — foreground only\n",
            "- `", stringify!($name), "!((FG, STYLE), ..)` — foreground + style\n",
            "- `", stringify!($name), "!((FG, BG, STYLE), ..)`", " — foreground + background + style\n",
            "\n",
            "Color/style is only emitted when the target stream supports it (see [`HasColors`]).\n\n",
            "## Example\n",
            "```rust\n",
            "use ino_color::", stringify!($name), ";\n",
            "use ino_color::fg::Yellow;\n",
            "use ino_color::style::Italic;\n",
            stringify!($name), "!(Yellow, \"Hello\");\n",
            stringify!($name), "!((Yellow, Italic), \"Hello\");\n",
            "```\n",
        )]
        macro_rules! $name {
            // fg only
            (
                $dol fg:path,
                $dol ($dol param:tt)*
            ) => {{
                use $crate::AnsiSgr;
                use $crate::HasColors;
                use std::io::Write;
                let stream = $stream();
                let should_color = stream.has_colors();
                let mut lock = stream.lock();
                if should_color {
                    write!(
                        lock,
                        "\x1b[{}m",
                        <$dol fg as AnsiSgr>::ATTR
                    )
                    .unwrap();
                }
                write!(lock, $dol ($dol param)*).unwrap();
                if should_color {
                    write!(lock, "\x1b[0m").unwrap();
                }
                $crate::__ino_newline!($newline, lock);
            }};

            // fg + style
            (
                ($dol fg:path, $dol style:path),
                $dol ($dol param:tt)*
            ) => {{
                use $crate::AnsiSgr;
                use $crate::HasColors;
                use std::io::Write;
                let stream = $stream();
                let should_color = stream.has_colors();
                let mut lock = stream.lock();
                if should_color {
                    write!(
                        lock,
                        "\x1b[{};{}m",
                        <$dol fg as AnsiSgr>::ATTR,
                        <$dol style as AnsiSgr>::ATTR
                    )
                    .unwrap();
                }
                write!(lock, $dol ($dol param)*).unwrap();
                if should_color {
                    write!(lock, "\x1b[0m").unwrap();
                }
                $crate::__ino_newline!($newline, lock);
            }};

            // fg + bg + style
            (
                ($dol fg:path, $dol bg:path, $dol style:path),
                $dol ($dol param:tt)*
            ) => {{
                use $crate::AnsiSgr;
                use $crate::HasColors;
                use std::io::Write;
                let stream = $stream();
                let should_color = stream.has_colors();
                let mut lock = stream.lock();
                if should_color {
                    write!(
                        lock,
                        "\x1b[{};{};{}m",
                        <$dol fg as AnsiSgr>::ATTR,
                        <$dol bg as AnsiSgr>::ATTR,
                        <$dol style as AnsiSgr>::ATTR
                    )
                    .unwrap();
                }
                write!(lock, $dol ($dol param)*).unwrap();
                if should_color {
                    write!(lock, "\x1b[0m").unwrap();
                }
                $crate::__ino_newline!($newline, lock);
            }};
        }
    };
    // Repetition to create each named print macro
    (
        $( ($name:ident, $print_macro:path, $stream:path, $newline:tt) ),*
        $(,)?
    ) => {
        // pass `$`
        $(create_print_macro!(
            $name,
            $print_macro,
            $stream,
            $newline,
            $
        );)*
    };
}

create_print_macro! {
    (cprint, std::print, std::io::stdout, false),
    (cprintln, std::println, std::io::stdout, true),
    (ceprint, std::eprint, std::io::stderr, false),
    (ceprintln, std::eprintln, std::io::stderr, true),
}

#[cfg(test)]
mod test {
    use super::*;
    use fg::*;
    use style::*;

    #[test]
    fn fg_only() {
        cprintln!(Blue, "hello");
        cprintln!(Yellow, "hello {}", "world");
    }

    #[test]
    fn fg_and_style() {
        cprintln!((Blue, Italic), "hello");
        cprintln!((Yellow, Bold), "hello {}", "world");
    }

    #[test]
    fn fg_bg_style() {
        cprintln!((Blue, bg::Red, Italic), "hello");
        cprintln!((Yellow, bg::Magenta, Bold), "hello {}", "world");
    }

    #[test]
    fn all_four_macros() {
        cprint!(Green, "no newline ");
        cprintln!(Green, "with newline");
        ceprint!(Cyan, "no newline ");
        ceprintln!(Cyan, "with newline");
    }

    #[test]
    fn format_traits() {
        cprintln!(Green, "{:?}", vec![123]);
        cprintln!(Green, "{:X}", 123);
    }
}
