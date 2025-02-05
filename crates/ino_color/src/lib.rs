pub mod has_colors;
pub use has_colors::HasColors;

use std::marker::PhantomData;

pub trait SgrParam<T> {
    const ATTR: &'static str;
}

pub struct ColorFG;
pub struct ColorBG;
pub struct Style;

/// Named ANSI SGR colors.
pub mod colors {
    macro_rules! lets_colors {
        ( $( $name:ident $fg:literal $bg:literal ),* $(,)? ) => { $(
            pub struct $name;
            impl crate::SgrParam<crate::ColorFG> for $name {
                const ATTR: &'static str = stringify!( $fg );
            }
            impl crate::SgrParam<crate::ColorBG> for $name {
                const ATTR: &'static str = stringify!( $bg );
            }
        )* }
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
}

/// Commonly recognized and used ANSI SGR attributes.
pub mod styles {
    macro_rules! lets_styles {
        ( $( $name:ident $attr:literal ),* $(,)? ) => { $(
            pub struct $name;
            impl crate::SgrParam<crate::Style> for $name {
                const ATTR: &'static str = stringify!( $attr );
            }
        )* }
    }
    lets_styles! {
        Default 10,
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
}

enum ShouldColorize<'obj, OBJ> {
    Yes( &'obj OBJ ),
    No( &'obj OBJ ),
}

impl<OBJ> ShouldColorize<'_, OBJ> {
    #[ inline ]
    fn get( &self ) -> &OBJ {
        match self {
            Self::Yes( o ) => o,
            Self::No( o ) => o,
        }
    }

    #[ inline ]
    fn should_colorize( &self ) -> bool {
        match self {
            Self::Yes(_) => true,
            Self::No(_) => false,
        }
    }
}

/// Add colors to some object. The color and style information
/// is embedded in its type, cool!
#[ repr( transparent ) ]
pub struct Painter<'painter, OBJ, SGR, KIND> {
    object: ShouldColorize<'painter, OBJ>,
    _phantom: PhantomData<(SGR, KIND)>,
}

impl<'painter, OBJ, SGR, KIND> Painter<'painter, OBJ, SGR, KIND>
where
    OBJ: 'painter,
    SGR: SgrParam<KIND>,
{
    #[ inline ]
    fn new( object: &'painter OBJ, colorize: bool ) -> Self {
        let object = match colorize {
            true => ShouldColorize::Yes( object ),
            false => ShouldColorize::No( object ),
        };
        Self { object, _phantom: PhantomData }
    }
}

macro_rules! impl_painter {
    // $trait : a trait to be implemented, repeated
    // $(,) : allow trailling comma
    ( $( $trait:path ),* $(,)? ) => { $(
        impl<OBJ, SGR, KIND> $trait for Painter<'_, OBJ, SGR, KIND>
        where
            OBJ: $trait,
            SGR: SgrParam<KIND>
        {
            fn fmt( &self, f: &mut std::fmt::Formatter<'_> )
                -> std::fmt::Result
            {
                // Of course it's the right use case for macro
                macro_rules! snippet {
                    () => { <OBJ as $trait>::fmt( self.object.get(), f )?; }
                }
                if self.object.should_colorize() {
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
    } }
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

/// Add colors and styles to output. This implementation uses generics heavily.
///
/// # Note
///
/// Background coloring is **not yet implemented** because I don't need them, yet.
///
/// # Examples
///
/// ```rust
/// use ino_color::InoColor;
/// use ino_color::colors::*;
/// use ino_color::styles::*;
///
/// let _ = "Hello Fancy".fg::<Yellow>();
///
/// // It's also chainable!
/// let _ = "Savoy blue"
///     .fg::<Blue>()
///     .style::<Italic>();
/// ```
pub trait InoColor
where
    Self: Sized
{
    #[ doc = METHOD_NOTE!( fg ) ]
    #[ inline ]
    fn fg<F: SgrParam<ColorFG>>( &self )
        -> Painter<Self, F, ColorFG>
    {
        Painter::new( self, should_colorize_snippet!() )
    }

    #[ doc = METHOD_NOTE!( style ) ]
    #[ inline ]
    fn style<S: SgrParam<Style>>( &self )
        -> Painter<Self, S, Style>
    {
        Painter::new( self, should_colorize_snippet!() )
    }

    #[ inline ]
    fn fg_always<F: SgrParam<ColorFG>>( &self )
        -> Painter<Self, F, ColorFG>
    {
        Painter::new( self, true )
    }

    #[ inline ]
    fn style_always<S: SgrParam<Style>>( &self )
        -> Painter<Self, S, Style>
    {
        Painter::new( self, true )
    }
}

impl<T: Sized> InoColor for T {}

#[ cfg( test ) ]
mod test {

    use super::*;
    use colors::*;
    use styles::*;

    #[ test ]
    fn print_something_to_see_theres_no_automated_tests() {
        println!( "{:?}", "wooo".fg::<Blue>() );
        println!( "{}", "uh".fg::<Yellow>().style::<Italic>() );
    }

}
