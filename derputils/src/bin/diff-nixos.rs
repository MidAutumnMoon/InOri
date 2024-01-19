use std::path::Path;
use std::process::Command;

use anyhow::{
    Result,
    Context,
    bail
};
use once_cell::sync::Lazy;

static SYSTEM_PROFILE: Lazy<&Path> = Lazy::new( || {
    Path::new( "/nix/var/nix/profiles/system" )
} );


fn main() -> Result<()> {

    if ! SYSTEM_PROFILE.exists() {
        bail! (
            "NixOS system profile not found. \
            Not running on NixOS?"
        )
    }

    let output = Command::new( "nix" )
        .arg( "profile" )
        .arg( "diff-closures" )
        .arg( "--profile" )
        .arg( SYSTEM_PROFILE.as_os_str() )
        .output()
        .with_context( || {
            "Failed to run nix"
        } )?;

    if ! output.status.success() {
        let message = format! {
            "Command nix returned non zero code. Stderr:\n\n{}",
            String::from_utf8_lossy( &output.stderr )
        };
        bail!( message )
    }

    print! {
        "{}",
        String::from_utf8_lossy( &output.stdout )
    }

    Ok( () )

}
