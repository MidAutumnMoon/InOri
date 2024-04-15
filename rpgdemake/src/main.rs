//! Notes on RPG Maker encryption
//!
//!
//! ## About the encrypting method
//!
//! RPG Maker uses a deadly naive encryption method.
//! Instead of a full content encryption, it XOR-s first
//! [`ENCRYPTION_LEN`] bytes of original file with the
//! encryption key of the same length, and leaviung
//! the remaning contents untouched.
//!
//!
//! ## About the encryption key
//!
//! The key can be found in field "encryptionKey"
//! of "System.json", which is typically in plain text,
//! but also can be compressed with lz4(?).
//!
//! The key is stored in a string, of which each pair of
//! **two** characters represent a hex value, and the total
//! number of such pair is equal to [`ENCRYPTION_LEN`],
//! resulting the total length of that string to be
//! *2\*[`ENCRYPTION_LEN`]*.


#[ global_allocator ]
static ALLOC: jemalloc::Jemalloc = jemalloc::Jemalloc;


use std::path::PathBuf;

use tracing::debug;

use clap::Parser;

use anyhow::{
    ensure,
    bail,
};

use colored::Colorize;

mod asset;

mod key;

mod walkdir;


/// Average amount of assets in a RPG Maker game.
/// As its name says, it's eyeballed.
const EYEBALLED_AVERAGE: usize = 1024;

const CONCURRENT_TASKS: usize  = 1024;


/// A simple CLI tool for batch decrypting
/// RPG Maker MV/MZ/XP assets.
#[ derive( Parser, Debug ) ]
struct CmdOpts {
    /// The path of
    game_directory: PathBuf,

    /// Brute force the decryption even the GAME_DIRECTORY
    /// not fitting RPG Maker game layout.
    #[ arg( long, short, default_value_t = false ) ]
    force: bool,

    /// Supply a key in force mode.
    #[ arg( long, short ) ]
    key: Option<String>,
}


#[ derive( Debug ) ]
struct DemakePlan {
    system_json: PathBuf,
    search_dirs: Vec<PathBuf>,
}



#[ tokio::main ]
async fn main() -> anyhow::Result<()> {

    // Initialize tracing

    ino_tracing::init_tracing_subscriber();


    // Parse CmdOpts

    let cmd_opts @ CmdOpts {
        game_directory,
        ..
    } = &CmdOpts::parse();

    debug!( ?cmd_opts );


    // Increase NOFILE

    debug!( "increase NOFILE rlimit" );

    rlimit::increase_nofile_limit( 20480 )?;


    // Probe game directory layout

    debug!( "probing directory layout" );

    ensure! { &game_directory.try_exists()?,
        "\"{}\" doesn't exists",
        &game_directory.display()
    };

    ensure! { &game_directory.is_dir(),
        "\"{}\" isn't a directory",
        &game_directory.display()
    };

    let plan = {
        let root = game_directory;

        let search_dirs;
        let system_json;

        // This is as far as where MV and MZ differs
        if root.join( "www" ).is_dir() {
            // MV
            system_json = root.join( "www/data/System.json" );
            search_dirs = Vec::from( &[
                root.join( "www/img" ),
                root.join( "www/audio" ),
            ] )
        } else {
            // MZ
            system_json = root.join( "data/System.json" );
            search_dirs = Vec::from( &[
                root.join( "img" ),
                root.join( "audio" ),
            ] );
        }

        DemakePlan { system_json, search_dirs }
    };

    debug!( ?plan );


    // Get encryption key

    debug!( "read encryption key" );

    let key = {
        use std::sync::Arc;
        use tokio::fs::read_to_string;
        use key::EncryptionKey;

        ensure!{ &plan.system_json.is_file(),
            "System.json doesn't exist"
        };

        let key = EncryptionKey::parse_json( {
            &read_to_string( &plan.system_json ).await?
        } )?;

        let key = match key {
            Some( k ) => k,
            None => bail!( "No key found, maybe not encrypted?" ),
        };

        Arc::new( key )
    };

    debug!( ?key );


    // Collect files to decrypt

    debug!( "collect files to decrypt" );

    let demake_files = {
        let mut files =
            Vec::with_capacity( 2 * EYEBALLED_AVERAGE );
        for sd in &plan.search_dirs {
            files.append(
                &mut walkdir::find_all( &sd )?
            )
        }
        files
    };

    debug!( ?demake_files, "all found files" );

    println! { "{}",
        format!(
            "{} files to be decrypted",
            demake_files.len()
        ).blue()
    };


    // Fire tasks

    debug!( "vroom vroom on decrypting" );

    use std::io::Write;

    use itertools::Itertools;

    use tokio::task::JoinSet;


    let total_tasks = demake_files.len();
    let mut finished_tasks = 0;

    let mut stdout = {
        std::io::stdout().lock()
    };

    // Split files into smaller batches for controlled
    // memory usage.
    for file_batch in
        &demake_files.iter().chunks( CONCURRENT_TASKS )
    {

        let mut tasks = JoinSet::new();

        for path in file_batch {
            let path = path.to_owned();
            let key = key.clone();
            tasks.spawn( async move {
                use asset::Asset;
                let asset = Asset::from_file( &path, &key ).await?;
                asset.write_decrypted().await?;
                anyhow::Result::<PathBuf>
                    ::Ok( asset.origin )
            } );
        }

        while let Some( result ) = tasks.join_next().await {
            use colored::Colorize;

            let message = match result {
                Err( e ) => format!( "async task failed {e}" ).red(),

                Ok( returns ) => match returns {
                    Ok( p ) => format! { "decryted \"{}\"",
                        p.display()
                    }.blue(),

                    Err( e ) =>
                        format!( "failed decrypt \"{e}\"" ).red()
                }
            };

            let _ = writeln!{ stdout,
                "{}/{total_tasks} {message}",
                finished_tasks + 1
            };

            finished_tasks += 1;
        }

    }


    Ok(())

}
