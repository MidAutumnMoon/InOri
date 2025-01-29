use std::path::PathBuf;

use tracing::debug;

use anyhow::ensure;
use anyhow::bail;
use anyhow::Context;

mod key;
mod finder;
mod task;
mod lore;
mod project;

use project::EngineRev;

/// A simple CLI tool for batch decrypting RPG Maker MV/MZ assets.
#[ derive( clap::Parser, Debug ) ]
struct CliOpts {
    /// Path to the directory containing the game.
    game_dir: PathBuf,
}

struct App {}

impl App {
    fn run_with( cliopts: CliOpts ) -> anyhow::Result<()> {
        todo!()
    }
}

fn main() -> anyhow::Result<()> {

    // Initialize tracing

    ino_tracing::init_tracing_subscriber();


    // Parse CLI options

    let cliopts = < CliOpts as clap::Parser >::parse();

    debug!( ?cliopts );


    // Increase NOFILE

    debug!( "increase NOFILE rlimit" );

    rlimit::increase_nofile_limit( u64::MAX )?;


    // Setup & sanity checks

    debug!( "Probe game engine revision" );

    let engine_rev = EngineRev::probe_revision( &cliopts.game_dir )
        .context( "Failed to understand game's engine revision" )?
    ;

    let ( system_json, resource_dirs ) = {
        let root = {
            let dir = &cliopts.game_dir;
            if dir.join( "www" ).try_exists()? {
                // If has "www", this should be a MV game
                dir.join( "www" )
            } else {
                // If "www" not presented, this should be a MZ game.
                dir.to_owned()
            }
        };
        let system_json = root
            .join( "data" )
            .join( "System.json" )
        ;
        let resource_dirs = vec![
            root.join( "img" ),
            root.join( "audio" ),
        ];
        ( system_json, resource_dirs )
    };

    debug!( ?system_json, ?resource_dirs );


    // Get encryption key

    debug!( "try read encryption key" );

    let enc_key = {
        ensure!{ system_json.is_file(),
            "System.json doesn't exist at \"{}\"",
            system_json.display()
        };

        let key = key::Key::parse_json(
            &std::fs::read_to_string( system_json )?
        )?;

        match key {
            Some( k ) => k,
            None => bail!(
                "System.json does not contain encryption key, maybe not encrypted?"
            ),
        }
    };

    debug!( ?enc_key );


    // Collect files to decrypt

    debug!( "collect files to decrypt" );

    let files = {

        use anyhow::Result as AResult;

        let files: Vec<PathBuf> = resource_dirs.iter()
            .map( |p| finder::find_all( p ) )
            .collect::< AResult<Vec<_>> >()?
            .into_iter()
            .flatten().collect()
        ;

        debug!( ?files, "all found files" );

        files
    };

    task::TaskRunner::new(
        &files,
        Box::leak( Box::new( enc_key ) )
    )?;

    Ok(())

}
