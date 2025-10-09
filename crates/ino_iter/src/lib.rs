use std::iter::Filter;

/// Some iterator extensions.
pub trait InoIter: Iterator {
    /// Alias of [`Iterator::filter`] with a more intuitive name.
    /// Leaving only items for which `pred` returns `true`.
    ///
    /// See also `select` method from Ruby: <https://docs.ruby-lang.org/en/3.4/Enumerable.html#method-i-select>
    #[inline]
    fn select<P>(self, pred: P) -> Filter<Self, P>
    where
        Self: Sized,
        P: FnMut(&Self::Item) -> bool,
    {
        self.filter(pred)
    }

    /// The inverse of [`Self::select`] with a more intuitive name.
    /// Remove(aka reject) items for which `pred` returns `true`.
    ///
    /// See also `reject` method from Ruby: <https://docs.ruby-lang.org/en/3.4/Enumerable.html#method-i-reject>
    #[inline]
    fn reject<P>(
        self,
        mut pred: P,
    ) -> Filter<Self, impl FnMut(&Self::Item) -> bool>
    where
        Self: Sized,
        P: FnMut(&Self::Item) -> bool,
    {
        self.filter(move |e| !pred(e))
    }
}

impl<T> InoIter for T where T: Iterator + ?Sized {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select() {
        let nums = vec![1, 2, 3, 4];
        // select odd numbers
        assert_eq!(
            vec![1, 3],
            nums.into_iter().select(|n| n % 2 != 0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_reject() {
        let nums = vec![1, 2, 3, 4];
        // reject even numbers
        assert_eq!(
            vec![1, 3],
            nums.into_iter().reject(|n| n % 2 == 0).collect::<Vec<_>>()
        );
    }
}
