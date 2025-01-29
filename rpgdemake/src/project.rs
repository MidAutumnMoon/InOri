use std::path::Path;
use std::path::PathBuf;

use anyhow::ensure;
use tracing::debug;

/// The revision of RPG Maker engine.
#[ derive( Debug ) ]
pub enum EngineRev {
    /// MV, the older one
    MV( PathBuf ),
    /// MV, the newer one
    MZ( PathBuf ),
}

impl EngineRev {
    #[ tracing::instrument ]
    pub fn probe_revision<P>( root: &P ) -> anyhow::Result<Self>
    where
        P: ToOwned<Owned = PathBuf> + std::fmt::Debug
    {
        debug!( "Probe engine revision from directory" );

        let root = root.to_owned();

        ensure! { root.is_dir(),
            "{} is not a directory", root.display()
        };
        ensure! { root.join( "locales" ).try_exists()?,
            "Game folder doesn't contains necessary files to be recognized \
            as a RPG Maker game. Maybe the directory is wrong, \
            it's not a RPG Maker MV/MZ game, or the files are packed into the exe."
        };

        if root.join( "www" ).try_exists()? {
            Ok( Self::MV( root ) )
        } else if root.join( "img" ).try_exists()? {
            Ok( Self::MZ( root ) )
        } else {
            anyhow::bail!( "Can't probe the engine revision from directory" )
        }
    }

    pub fn get_img_dir( &self ) -> PathBuf {
        match self {
            Self::MV( p ) => p.join( "www" ).join( "img" ),
            Self::MZ( p ) => p.join( "img" ),
        }
    }

    pub fn get_audio_dir( &self ) -> PathBuf {
        match self {
            Self::MV( p ) => p.join( "www" ).join( "audio" ),
            Self::MZ( p ) => p.join( "audio" ),
        }
    }

    pub fn get_data_dir( &self ) -> PathBuf {
        match self {
            Self::MV( p ) => p.join( "www" ).join( "data" ),
            Self::MZ( p ) => p.join( "data" ),
        }
    }
}
