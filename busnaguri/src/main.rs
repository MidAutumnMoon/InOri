use anyhow::ensure;
use anyhow::Context;
use anyhow::Result as AnyResult;
use tap::Pipe;
use tap::Tap;
use tracing::debug;
use tracing::trace;

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

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

            let mut cmd = Command::new( "systemd-run" );
            cmd.env_clear();

            cmd
                .args( [ "--user", "--scope", "--collect" ] )
                .arg( "--" )
                .arg( cmd_path ).args( args );

            let envs = match UserEnv::new() {
                Ok( v ) => v,
                Err( err ) => break 'out Err( format!(
                    "Failed to get user environment, caused by: {err:?}" ) )
            };

            for ( name, val ) in envs {
                eprintln!( "{name}={val}" );
                cmd.env( name, val );
            }

            let cmd_ret = cmd.output().await;

            match cmd_ret {
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

#[ derive( Debug ) ]
struct UserEnv {
    envs: Vec<( String, String )>
}

impl Iterator for UserEnv {
    type Item = ( String, String );
    fn next( &mut self ) -> Option<Self::Item> {
        self.envs.pop()
    }
}

impl UserEnv {
    #[ allow( clippy::unwrap_in_result ) ]
    #[ allow( clippy::unwrap_used ) ]
    #[ allow( clippy::undocumented_unsafe_blocks ) ]
    fn new() -> AnyResult<Self> {
        let def = unsafe { geteuid() }
            .pipe( |val| val.to_string() )
            .pipe( |val| format!( "/run/user/{val}" ) );
        let runtimedir = std::env::var_os( "XDG_RUNTIME_DIR" )
            .unwrap_or_else( || def.into() )
            .to_str().unwrap().to_owned();
        let mut cmd = Command::new( "systemctl" );
        cmd.env( "XDG_RUNTIME_DIR", runtimedir );
        cmd.arg( "--user" ).arg( "show-environment" );
        let output = cmd.output()?;
        ensure!( output.status.success(),
            r#"Failed to run systemctl "{}""#,
            String::from_utf8_lossy( &output.stderr )
        );
        String::from_utf8( output.stdout )?
            .lines()
            .filter_map( |line| line.split_once( '=' ) )
            .map( |val| ( val.0.to_owned(), val.1.to_owned() ) )
            .collect::<Vec<_>>()
            .pipe( |val| Self { envs: val } )
            .pipe( Ok )
    }
}

#[ link( name = "c" ) ]
unsafe extern "C" {
    fn geteuid() -> u32;
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
