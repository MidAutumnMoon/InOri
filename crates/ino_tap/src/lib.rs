use tap::Tap;

/// Extension trait to [`Tap`] that adds few commonly used
/// pattern.
pub trait TapExt: Tap {

    /// Trace self using [`tracing::trace`]
    #[ allow( clippy::inline_always ) ]
    #[ inline( always ) ]
    #[ must_use ]
    fn tap_trace( self ) -> Self
    where
        Self: std::fmt::Debug
    {
        self.tap( |it| tracing::trace!( ?it ) )
    }

}

impl<T> TapExt for T where T: Sized {}
