//! Coloring the terminal output.
//!
//! # Basic Usage
//!
//! ```rust
//! // This's the trait that adds coloring methods.
//! use ino_color::InoColor;
//!
//! // These two modules contain predefined colors and styles.
//! // As a personal preferrence, wildcard import is avoided,
//! // even though doing so makes the function call looks funnier.
//! use ino_color::fg;
//! use ino_color::style;
//!
//! // The most basic usage
//! println!(
//!     "{}", "Hello Fancy".fg::<fg::Yellow>()
//! );
//!
//! // It's also chainable!
//! println!(
//!     "{}", "Savoy blue".fg::<fg::Blue>().style::<style::Italic>()
//! );
//!
//! // In fact, anything which implements `std::fmt` traits can be colored.
//! println!( "{:?}", vec![123].fg::<fg::Green>() );
//! println!( "{:X}", 123.fg::<fg::Green>() );
//! ```

pub use has_colors::HasColors;
pub mod has_colors;

use std::marker::PhantomData;

/// An attribute in the [ANSI SGR](https://w.wiki/DBZ2) list.
pub trait AnsiSgr {
    const ATTR: &'static str;
}

/// The corresponding attribute is for *foreground color*.
pub trait FG : AnsiSgr {}
/// The corresponding attribute is for *background color*.
pub trait BG : AnsiSgr {}
/// The corresponding attribute is for attributes which mainly
/// effects the *style* of output, such as italic or bold.
pub trait Style : AnsiSgr {}

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

enum ShouldColorize<'obj, OBJ> {
    Yes( &'obj OBJ ),
    No( &'obj OBJ ),
}

/// Add colors to some object. The color and style information
/// is embedded in its type, cool!
#[ repr( transparent ) ]
pub struct Painter<'painter, OBJ, SGR> {
    object: ShouldColorize<'painter, OBJ>,
    _phantom: PhantomData<(SGR, )>,
}

impl<'painter, OBJ, SGR> Painter<'painter, OBJ, SGR>
where
    OBJ: 'painter,
    SGR: AnsiSgr
{
    #[ inline ]
    const fn new<const COLOR: bool>( object: &'painter OBJ ) -> Self {
        let object = if COLOR {
            ShouldColorize::Yes( object )
        } else {
            ShouldColorize::No( object )
        };
        Self { object, _phantom: PhantomData }
    }

    #[ inline ]
    const fn should_colorize( &self ) -> bool {
        matches!( self.object, ShouldColorize::Yes(_) )
    }

    #[ inline ]
    const fn get_inner( &self ) -> &OBJ {
        use ShouldColorize::{ Yes, No };
        match self.object {
            Yes( o ) | No( o ) => o
        }
    }
}

macro_rules! impl_painter {
    // $trait : a trait to be implemented, repeated
    // $(,) : allow trailling comma
    ( $( $trait:path ),* $(,)? ) => { $(
        impl<OBJ, SGR> $trait for Painter<'_, OBJ, SGR>
        where
            OBJ: $trait,
            SGR: AnsiSgr
        {
            fn fmt( &self, f: &mut std::fmt::Formatter<'_> ) -> std::fmt::Result {
                // Of course it's the right use case for macro
                macro_rules! snippet {
                    () => { <OBJ as $trait>::fmt( self.get_inner(), f )?; }
                }
                if self.should_colorize() {
                    f.write_str( "\x1b[" )?;
                    f.write_str( SGR::ATTR )?;
                    f.write_str( "m" )?;
                    snippet!();
                    f.write_str( "\x1b[0m" )?;
                } else {
                    snippet!();
                }
                Ok(())
            }
        }
    )* }
}

impl_painter! {
    std::fmt::Display,
    std::fmt::Debug,
    std::fmt::UpperHex,
    std::fmt::LowerHex,
    std::fmt::Binary,
    std::fmt::UpperExp,
    std::fmt::LowerExp,
    std::fmt::Octal,
    std::fmt::Pointer,
}

macro_rules! should_colorize_snippet {
    () => { {
        use crate::HasColors;
        use std::io::stdout;
        use std::io::stderr;
        stdout().has_colors() && stderr().has_colors()
    } };
    ( $self:ident ) => {
        if should_colorize_snippet!() {
            Painter::new::<true>( $self )
        } else {
            Painter::new::<false>( $self )
        }
    };
}

macro_rules! METHOD_NOTE { ( $name:ident ) => {
    concat!(
        "\
            # Note \n\
            This method will do a [`HasColors`] check behind the scene \
            on **both** [`std::io::Stdin`] and [`std::io::Stdout`], \
            and only enables color if both checks passed. \
            \n\n\
            The check involves reading environment variable and \
            obtain locks so it can be expensive to doing rapidly. \
            It's generally recommended to cache the colored string. \
            \n\n\
            If the check is undesired, use \
        ",
        "[`Self::",
        stringify!( $name ),
        "_always`] instead to always enable colors."
    )
} }

/// Have methods for coloring things.
///
/// # Note
/// Background coloring is **not yet implemented** because I don't need them, yet.
pub trait InoColor
where
    Self: Sized
{
    #[ doc = METHOD_NOTE!( fg ) ]
    #[ inline ]
    fn fg<F: FG>( &self ) -> Painter<'_, Self, F> {
        should_colorize_snippet!( self )
    }

    #[ doc = METHOD_NOTE!( style ) ]
    #[ inline ]
    fn style<S: Style>( &self ) -> Painter<'_, Self, S> {
        should_colorize_snippet!( self )
    }

    #[ inline ]
    fn fg_always<F: FG>( &self ) -> Painter<'_, Self, F> {
        Painter::new::<true>( self )
    }

    #[ inline ]
    fn style_always<S: Style>( &self ) -> Painter<'_, Self, S> {
        Painter::new::<true>( self )
    }
}

impl<T: Sized> InoColor for T {}

#[ cfg( test ) ]
mod test {

    use super::*;
    use fg::*;
    use style::*;

    #[ test ]
    fn print_something_to_see_theres_no_automated_tests() {
        println!( "{:?}", "wooo".fg::<Blue>() );
        println!( "{}", "uh".fg::<Yellow>().style::<Italic>() );
        println!( "{:x}", 123.fg::<Green>() );
    }

}
