//! Ruby-style [`select`](InoIter::select) and [`reject`](InoIter::reject)
//! methods for iterators.
//!
//! Both are lazy, like [`Iterator::filter`].
//! Predicates take `&Self::Item`; iterators over references pass `&&T`.
//!
//! # Example
//!
//! ```
//! use ino_iter::InoIter as _;
//!
//! fn is_even(number: &i32) -> bool {
//!     number % 2 == 0
//! }
//!
//! assert_eq!(
//!     (1..=4).select(is_even).collect::<Vec<_>>(),
//!     vec![2, 4],
//! );
//! assert_eq!(
//!     (1..=4).reject(is_even).collect::<Vec<_>>(),
//!     vec![1, 3],
//! );
//! ```

use std::iter::Filter;

/// Ruby-style filtering methods for iterators.
pub trait InoIter: Iterator {
    /// Creates an iterator that yields items for which `pred` returns `true`.
    ///
    /// Equivalent to [`Iterator::filter`]. Ruby calls this
    /// [`Enumerable#select`](https://docs.ruby-lang.org/en/3.4/Enumerable.html#method-i-select).
    #[inline]
    fn select<P>(self, pred: P) -> Filter<Self, P>
    where
        Self: Sized,
        P: FnMut(&Self::Item) -> bool,
    {
        self.filter(pred)
    }

    /// Creates an iterator that yields items for which `pred` returns `false`.
    ///
    /// Named after Ruby's
    /// [`Enumerable#reject`](https://docs.ruby-lang.org/en/3.4/Enumerable.html#method-i-reject).
    #[inline]
    fn reject<P>(
        self,
        mut pred: P,
    ) -> Filter<Self, impl FnMut(&Self::Item) -> bool>
    where
        Self: Sized,
        P: FnMut(&Self::Item) -> bool,
    {
        self.filter(move |element| !pred(element))
    }
}

impl<T> InoIter for T where T: Iterator + ?Sized {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select() {
        let nums = vec![1, 2, 3, 4];
        // select odd numbers
        assert_eq!(
            vec![1, 3],
            nums.into_iter().select(|n| n % 2 != 0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn reject() {
        let nums = vec![1, 2, 3, 4];
        // reject even numbers
        assert_eq!(
            vec![1, 3],
            nums.into_iter().reject(|n| n % 2 == 0).collect::<Vec<_>>()
        );
    }
}
