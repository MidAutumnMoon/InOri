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


use std::{
    thread::available_parallelism,
    path::PathBuf,
};

use tracing::{
    debug,
    debug_span,
};

use clap::Parser;

use anyhow::{
    ensure,
    bail,
};

use itertools::Itertools;


mod asset;
mod key;
mod finder;


/// Average amount of assets in a RPG Maker game.
/// As its name says, it's eyeballed.
const EYEBALLED_AVERAGE: usize = 1024;


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

    /// Use this key instead of find one in GAME_DIRECTORY.
    #[ arg( long, short ) ]
    key: Option<String>,

    /// Number of threads used to decrypt assets.
    #[ arg(
        long, short,
        default_value_t = 4 *
            available_parallelism().unwrap().get()
    ) ]
    threads: usize,
}


#[ derive( Debug ) ]
struct ResourceLocation {
    system_json: PathBuf,
    asset_dirs: Vec<PathBuf>,
}


fn main() -> anyhow::Result<()> {

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

    let location = {
        let root = game_directory;

        let asset_dirs;
        let system_json;

        // This is as far as where MV and MZ differs
        if root.join( "www" ).is_dir() {
            // MV
            system_json = root.join( "www/data/System.json" );
            asset_dirs = Vec::from( &[
                root.join( "www/img" ),
                root.join( "www/audio" ),
            ] )
        } else {
            // MZ
            system_json = root.join( "data/System.json" );
            asset_dirs = Vec::from( &[
                root.join( "img" ),
                root.join( "audio" ),
            ] );
        }

        ResourceLocation { system_json, asset_dirs }
    };

    debug!( ?location );


    // Get encryption key

    debug!( "try read encryption key" );

    let encryption_key = {
        use std::fs::read_to_string;
        use std::sync::Arc;
        use key::EncryptionKey;

        let ResourceLocation { system_json, .. } = &location;

        ensure!{ system_json.is_file(),
            "System.json doesn't exist at \"{}\"",
            system_json.display()
        };

        let key = EncryptionKey::parse_json( {
            &read_to_string( system_json )?
        } )?;

        match key {
            Some( k ) => Arc::new( k ),
            None => bail!( "No key found, maybe not encrypted?" ),
        }
    };

    debug!( ?encryption_key );


    // Collect files to decrypt

    debug!( "collect files to decrypt" );

    let found_files = {
        let mut files =
            Vec::with_capacity( 2 * EYEBALLED_AVERAGE );
        for ad in &location.asset_dirs {
            files.append( &mut finder::find_all( ad )? )
        }
        files
    };

    debug!( ?found_files, "all found files" );

    {
        use colored::Colorize;

        let message = format! {
            "{} files to be decrypted",
            found_files.len()
        }.blue();

        println! { "{message}" };
    }



    // Fire tasks

    debug!( "vroom vroom on decrypting" );

    std::thread::scope( |scope| {

        use std::sync::mpsc;


        // TODO: use crossbeam::deque to maximize efficiency

        enum TaskStatus {
            Ok( PathBuf ),
            Err( PathBuf, anyhow::Error )
        }

        let ( origin_sender, receiver ) =
            mpsc::channel::< TaskStatus >();


        let total_tasks = found_files.len();

        let split_tasks = {
            let total = found_files.len();
            found_files.into_iter()
                .chunks( total.div_ceil( cmd_opts.threads ) )
        };


        for file_queue in split_tasks.into_iter() {

            let file_queue = file_queue.collect_vec();

            debug!( "fire up thread with queue of size {}",
                file_queue.len()
            );

            let encryption_key = encryption_key.clone();
            let sender = origin_sender.clone();

            scope.spawn( move || {

                let _span =
                    debug_span!( "thread", id = std::process::id() )
                    .entered()
                ;

                for path in file_queue.into_iter() {

                    let _span =
                        debug_span!( "task", ?path ).entered();

                    use asset::Asset as As;

                    let asset = match
                        As::from_file( &path, &encryption_key)
                    {
                        Ok( asset ) => asset,
                        Err( err ) => {
                            let _ = sender.send(
                                TaskStatus::Err( path, err )
                            );
                            break
                        },
                    };

                    let _ = match asset.write_decrypted() {
                        Ok(_) => sender.send(
                            TaskStatus::Ok( path )
                        ),
                        Err( err ) => sender.send(
                            TaskStatus::Err( path, err )
                        ),
                    };
                }

            } ); // END scope.spanw

        }; // END file_queue

        drop( origin_sender );


        use std::io::Write;

        let mut stdout = { std::io::stdout().lock() };

        for ( count, message ) in receiver.iter().enumerate() {

            use colored::Colorize;
            use TaskStatus as T;

            let message = match message {
                T::Ok( p ) =>
                    format!( "(ok) {}", p.display() ).blue(),
                T::Err( p, e ) =>
                    format!( "(err) {} {e}", p.display() ).red()
            };

            writeln!( stdout, "{}/{} {message}",
                count + 1,
                total_tasks
            ).unwrap();

        }

    } ); // END thread scope


    Ok(())

}
