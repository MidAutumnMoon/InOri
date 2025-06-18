use anyhow::Context;
use tap::Tap;
use tracing::debug;
use tracing::trace;

use std::path::Path;
use std::path::PathBuf;

const DEFAULT_NIX_STORE: &str = "/nix/store";

// fuck dbus
const SERVICE_NAME: &str = "im._418.Busnaguri";
const OBJECT_PATH: &str = "/Naguru";

/// A dbus service to run arbitrary(!!) commands.
#[ derive( clap::Parser, Debug ) ]
struct CliOpts {
    /// The path of nix store
    #[ arg( long, short, default_value=DEFAULT_NIX_STORE ) ]
    nix_store: PathBuf,

    /// Don't check if the command is from nix store before running it,
    /// potentially harmful to the system.
    #[ arg( long, default_value_t=false ) ]
    unsafe_skip_store_check: bool,
}

impl CliOpts {
    #[ inline ]
    fn parse() -> Self {
        <Self as clap::Parser>::parse()
    }
}

#[ derive( Debug, Clone ) ]
struct Naguru {
    nix_store: PathBuf,
    unsafe_skip_store_check: bool,
}

impl Naguru {
    #[ tracing::instrument ]
    fn new( cliopts: CliOpts ) -> Self {
        let CliOpts { nix_store, unsafe_skip_store_check } = cliopts;
        Self { nix_store, unsafe_skip_store_check }
    }

    #[ tracing::instrument( skip_all ) ]
    async fn serve( self ) -> anyhow::Result<()> {
        debug!( "Starting server" );
        let _c = zbus::connection::Builder::session()?
            .name( SERVICE_NAME )?
            .serve_at( OBJECT_PATH, self )?
            .build()
            .await?;
        std::future::pending::<()>().await;
        Ok(())
    }

    /// Check if `target` is a path from the nix store.
    #[ inline ]
    fn check_if_path_from_store( &self, target: &Path ) -> bool {
        target.starts_with( &self.nix_store )
    }
}

#[ zbus::interface( name = "im._418.busnaguri" ) ]
impl Naguru {

    #[ tracing::instrument( skip( self ) ) ]
    async fn exec( &self, cmd_path: String ) -> String {
        serde_json::json!( {
            "ok": false,
            "err_msg": "not implmented"
        } ).to_string()
    }

    #[ tracing::instrument( skip( self ) ) ]
    async fn exec_args(
        &self,
        cmd_path: String,
        // idoit kwin script can't do "sas"
        // the signature has to be "sav"
        args: Vec<zbus::zvariant::OwnedValue>
    ) -> String {
        debug!( ?cmd_path, "try execute command" );

        let cmd_path = PathBuf::from( cmd_path );

        let res: Result<(), String> = 'out: {
            use tokio::process::Command;

            if !self.unsafe_skip_store_check
                && !self.check_if_path_from_store( &cmd_path )
            {
                break 'out Err( "The given command is not from nix store".into() )
            }

            let args = {
                let mut accu = vec![];
                for a in args {
                    use zbus::zvariant::Value::Str;
                    if let Str( s ) = a.into() {
                        accu.push( s.to_string() );
                    } else {
                        break 'out Err( "Sig is not sav, but in fact sas. \
                            DBus sucks so here's the workaround.".into() )
                    }
                }
                accu
            };

            let cmd_res = Command::new( cmd_path )
                .args( args )
                .output()
                .await;

            match cmd_res {
                Ok( output ) => {
                    if !output.status.success() {
                        debug!( "Command failed" );
                        let msg = format!(
                            "Command exited error: \nstdout: {}\n stderr: {}",
                            String::from_utf8_lossy( &output.stdout ),
                            String::from_utf8_lossy( &output.stderr ),
                        );
                        break 'out Err( msg )
                    }
                },
                Err( err ) => {
                    debug!( "Can't run the command" );
                    let msg = format!( "Failed to run command: {err:?}" );
                    break 'out Err( msg )
                }
            }

            debug!( "Command succeed" );
            break 'out Ok(())
        };

        serde_json::json!( {
            // Whether the command succeed
            "ok": res.is_ok(),
            // If not `ok`, here is the reason
            "err_msg": res.err()
        } )
            .tap( |it| trace!( ?it ) )
            .to_string()
    }

    #[ tracing::instrument( skip( self ) ) ]
    async fn exec_args_env(&self) -> String {
        serde_json::json!( {
            "ok": false,
            "err_msg": "not implmented"
        } ).to_string()
    }
}

#[ tokio::main( flavor = "current_thread" ) ]
async fn main() -> anyhow::Result<()> {

    ino_tracing::init_tracing_subscriber();

    debug!( "prepare app" );

    let cliopts = CliOpts::parse();

    debug!( ?cliopts );

    let nagaru = Naguru::new( cliopts );

    debug!( ?nagaru );

    nagaru.serve().await
        .context( "Failed to launch dbus service" )?;

    Ok(())

}
