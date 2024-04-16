use std::path::{
    PathBuf,
    Path,
};

use anyhow::{
    ensure,
    bail
};

use itertools::Itertools;

use tracing::{
    debug,
    error,
};


/// Length of general RPG Maker encrypted file header.
pub const RPGMV_HEADER_LEN: usize = 16;

/// The stock RPGMV header.
pub const RPGMV_HEADER: [u8; RPGMV_HEADER_LEN] = [
    // R P G M V -- SIGNATURE in rpg_core.js
    0x52, 0x50, 0x47, 0x4d, 0x56,
    // padding
    0x00, 0x00, 0x00,
    // version sorta -- VER in rpg_core.js
    0x00, 0x03, 0x01,
    // padding
    0x00, 0x00, 0x00, 0x00, 0x00
];

/// The length of encryption key.
///
/// The default length is 16, but key of other sizes
/// hasn't been spotted in the wild. Referring to
/// rpg_core.js this apparently is the only allowed length.
pub const ENCRYPTION_KEY_LEN: usize = 16;

/// The length of encrypted portion of original file.
///
/// Since it's a simple byte-to-byte XOR operation,
/// the parted being encrypted is equal to [`ENCRYPTION_KEY_LEN`].
pub const ENCRYPTION_LEN: usize = ENCRYPTION_KEY_LEN;



#[ derive( Debug ) ]
pub struct Asset {
    decrypted: DecryptAsset,
    pub origin: PathBuf,
    pub target: PathBuf,
}

impl Asset {

    /// Construct [`Asset`] containing decrypted data.
    #[ tracing::instrument( skip(key) ) ]
    pub fn from_file(
        path: &Path,
        key: &crate::key::EncryptionKey,
    ) -> anyhow::Result<Self>
    {
        debug!( "working on asset file" );

        ensure! { path.is_file(),
            "\"{}\" is not file",
            path.display()
        };

        let target = {
            let ext = match Self::real_extension( path ) {
                Some( e ) => e,
                None => bail!( "No extension" )
            };
            let mut p = path.to_owned();
            p.set_extension( ext );
            p
        };

        debug!( "read file" );

        let decrypted = DecryptAsset::new(
            std::fs::read( path )?,
            key
        )?;

        Ok( Self {
            decrypted, target,
            origin: path.to_owned(),
        } )
    }


    /// Write the decrypted content to the new file.
    #[ tracing::instrument(
        skip_all,
        fields( ?self.origin, ?self.target )
    ) ]
    pub fn write_decrypted( &self )
        -> anyhow::Result<()>
    {
        debug!( "write decrypted file" );
        std::fs::write(
            &self.target,
            self.decrypted.get()
        )?;
        Ok(())
    }


    /// Canonicalize the extension of encrypted files
    /// to its real counterpart.
    #[ tracing::instrument ]
    pub fn real_extension( path: &Path )
        -> Option< &'static str >
    {
        debug!( "try fixing file extension" );
        let ext = match path.extension() {
            Some( e ) => e,
            None => {
                debug!( "no extension" );
                return None
            }
        };
        let ext = match ext.to_str() {
            Some( e ) => e,
            None => {
                debug!( "ignore failed OsStr convertion" );
                return None
            }
        };
        match ext {
            "rpgmvp" | "png_" => Some( "png" ),
            "rpgmvm" | "m4a_" => Some( "m4a" ),
            "rpgmvo" | "ogg_" => Some( "ogg" ),
            _ => { debug!( "no real extension found" ); None }
        }
    }

}



#[ derive( Debug ) ]
pub struct DecryptAsset {
    data: Vec<u8>,
}

impl DecryptAsset {
    #[ tracing::instrument( skip_all ) ]
    pub fn new(
        mut encrypted: Vec<u8>,
        key: &crate::key::EncryptionKey
    ) -> anyhow::Result< Self >
    {
        debug!( "decrypt data" );

        ensure! {
            encrypted.len() >= RPGMV_HEADER_LEN + ENCRYPTION_LEN,
            {
                error!( "invalid data format" );
                "Data is too small to be encrypted"
            }
        };

        if encrypted[..RPGMV_HEADER_LEN] != RPGMV_HEADER {
            bail!( "Invalid RPGMV file header" )
        }

        let mut content = encrypted
            .drain( RPGMV_HEADER_LEN.. )
            .collect_vec();

        key.get().iter().enumerate()
            .for_each( |(i, mask)| content[i] ^= mask );

        Ok( Self { data: content } )
    }


    pub fn get( &self ) -> &[u8] {
        &self.data
    }
}


#[ cfg( test ) ]
mod tests {

    use crate::key::EncryptionKey;
    use super::*;

    const SYSTEM_JSON: &str =
        include_str!( "../fixture/System.json" );

    const CLOUDS_PNG: &[u8] =
        include_bytes!( "../fixture/Clouds.png" );

    const CLOUDS_RPGMVP: &[u8] =
        include_bytes!( "../fixture/Clouds.rpgmvp" );


    fn key() -> EncryptionKey {
        EncryptionKey::parse_json( SYSTEM_JSON )
            .unwrap()
            .unwrap()
    }


    #[ test ]
    fn decrypt_succeed() {
        let a = DecryptAsset::new(
            CLOUDS_RPGMVP.into(), &key()
        ).unwrap();
        assert_eq!( a.get(), CLOUDS_PNG );
    }

    #[ test ]
    fn too_small() {
        assert! {
            DecryptAsset::new( vec![0], &key() ).is_err()
        }
    }

    #[ test ]
    fn invalid_rpgmv_header() {
        let mut broken = CLOUDS_RPGMVP.to_owned();
        broken[2] = 0x99;
        assert! {
            DecryptAsset::new( broken, &key() ).is_err()
        }
    }

}
