use rootcause::Report;
use rootcause::prelude::ResultExt;
use xshell::Shell;

/// Variant of the system Nix, notice that detsys Nix is not supported.
pub enum NixVariant {
    Nix,
    Lix,
}

pub fn nix_variant() -> rootcause::Result<NixVariant> {
    let shell = Shell::new()?;
    let version_output = xshell::cmd!(shell, "nix --version")
        .read()
        .context("Failed to run nix --version")?;
    println!("{version_output}");
    todo!()
}
