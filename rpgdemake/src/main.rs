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

use tracing::debug;

use clap::Parser;

use anyhow::{
    ensure,
    bail,
    Context,
};


mod asset;
mod key;
mod finder;
mod tasks;

use asset::Asset;


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

    let enc_key = {
        use std::fs::read_to_string;
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
            Some( k ) => k,
            None => bail!( "No key found, maybe not encrypted?" ),
        }
    };

    debug!( ?enc_key );


    // Collect files to decrypt

    debug!( "collect files to decrypt" );

    let assets: Vec<Asset> = {
        use colored::Colorize;

        let found_files = {
            let mut files = Vec::new();
            for ad in &location.asset_dirs {
                files.append( &mut finder::find_all( ad )? )
            }
            files
        };

        debug!( ?found_files, "all found files" );

        println! { "{}", format! {
            "{} files to be decrypted",
            found_files.len()
        }.blue() };

        type ResultVec = anyhow::Result< Vec<Asset> >;

        found_files.into_iter()
            .map( |p| Asset::new( &p, enc_key.clone() ) )
            .collect::<ResultVec>()
            .context( "Failed to make asset" )?
    };


    // Fire tasks

    debug!( "vroom vroom on decrypting" );

    tasks::submit_assets(
        assets,
        cmd_opts.threads
    );


    Ok(())

}
