use color_eyre::eyre;

#[expect(clippy::missing_errors_doc)]
pub trait EyreRootcauseBridge<T> {
    fn into_rootcause(self) -> rootcause::Result<T>;
}

impl<T> EyreRootcauseBridge<T> for eyre::Result<T> {
    fn into_rootcause(self) -> rootcause::Result<T> {
        Ok(self.map_err(|eyre_err| {
            // 1. Convert eyre::Report into a standard boxed error
            let std_err: Box<dyn std::error::Error + Send + Sync> =
                eyre_err.into();

            // 2. Wrap it into a rootcause report
            rootcause::report!(std_err)
        })?)
    }
}
