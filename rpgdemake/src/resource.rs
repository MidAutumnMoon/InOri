use std::path::{
    PathBuf,
    Path,
};

use anyhow::{
    ensure,
    bail,
    Context,
};

use tracing::debug;

use crate::key::EncryptionKey;


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



#[ derive( Debug, Clone ) ]
pub struct Resource {
    pub origin: PathBuf,
    pub target: PathBuf,
    pub encryption_key: EncryptionKey,
}

impl Resource {

    #[ tracing::instrument(
        name = "asset",
        skip(encryption_key)
    ) ]
    pub fn new( path: &Path, encryption_key: EncryptionKey )
        -> anyhow::Result< Self >
    {
        debug!( "new asset" );

        let target = {
            let ext = match Self::real_extension( path ) {
                Some( e ) => e,
                None => bail!(
                    "Can't find real extension for {}",
                    path.display()
                )
            };
            let mut p = path.to_owned();
            p.set_extension( ext );
            p
        };

        Ok( Self {
            origin: path.to_owned(),
            target,
            encryption_key,
        } )
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
pub struct DecryptResource {
    resource: Resource,
    decrypted: Vec<u8>,
}

impl DecryptResource {

    #[ tracing::instrument() ]
    pub fn new( resource: Resource )
        -> anyhow::Result< Self >
    {
        debug!( "prepare decrypt asset" );

        use std::io::{
            prelude::*,
            ErrorKind as IoErr,
        };

        debug!( "open asset file to read" );

        ensure! { resource.origin.is_file(),
            "{} is not a file",
            resource.origin.display()
        };

        let mut file = std::fs::File
            ::open( &resource.origin )
            .context( "Failed to open asset file" )?
        ;

        {
            debug!( "verify RPGMV header" );

            let mut header = [ 0; RPGMV_HEADER_LEN ];

            match file.read_exact( &mut header ) {
                Ok(_) => {},
                Err( err ) => match err.kind() {
                    IoErr::UnexpectedEof => bail!(
                        "File is too small to be encrypted"
                    ),
                    _ => bail!( err )
                }
            }

            debug!( ?header );

            if header != RPGMV_HEADER {
                bail!( "Invalid RPGMV encryption header" )
            }
        }

        debug!( "read remaning content" );

        // 300KiB the eyeballed average
        // It'd be smaller in general
        let mut content = Vec::with_capacity( 300 * 1024 );

        file.read_to_end( &mut content )?;

        ensure! { content.len() > ENCRYPTION_LEN,
            "Insufficient content for decryption"
        };

        resource.encryption_key.get()
            .iter().enumerate()
            .for_each( |(idx, mask)| content[idx] ^= mask );

        Ok( Self {
            resource,
            decrypted: Vec::from( content )
        } )
    }


    #[ tracing::instrument(
        skip_all,
        fields( ?self.resource )
    ) ]
    pub fn write_decrypted( &self )
        -> anyhow::Result< () >
    {
        debug!( "write decrypted asset" );
        std::fs::write(
            &self.resource.target,
            &self.decrypted
        )?;
        Ok(())
    }

}


#[ cfg( test ) ]
mod tests {

    use assert_fs::prelude::*;
    use assert_fs::TempDir;

    use crate::key::EncryptionKey;
    use super::*;

    const JSON: &str =
        include_str!( "../tests/fixture/System.json" );

    const CLOUDS_PNG: &[u8] =
        include_bytes!( "../tests/fixture/Clouds.png" );

    const CLOUDS_RPGMVP: &[u8] =
        include_bytes!( "../tests/fixture/Clouds.rpgmvp" );


    fn key() -> EncryptionKey {
        EncryptionKey::parse_json( JSON ).unwrap().unwrap()
    }


    #[ test ]
    fn decrypt_succeed() {
        let tmp = TempDir::new().unwrap();

        let f = tmp.child( "clouds.rpgmvp" );
        f.write_binary( CLOUDS_RPGMVP ).unwrap();

        let ass = Resource::new( &f.path(), key() ).unwrap();

        let d = DecryptResource::new( ass ).unwrap();

        assert_eq!( d.decrypted, CLOUDS_PNG );
    }


    #[ test ]
    fn too_small() {
        let tmp = TempDir::new().unwrap();

        let f = tmp.child( "invalid.rpgmvp" );
        f.touch().unwrap();

        let ase = Resource::new( f.path(), key() ).unwrap();

        assert! {
            DecryptResource::new( ase ).is_err()
        }
    }


    #[ test ]
    fn invalid_rpgmv_header() {
        let tmp = TempDir::new().unwrap();

        let f = tmp.child( "clouds.rpgmvp" );

        let mut clouds = CLOUDS_RPGMVP.to_owned();

        clouds[3] = 0x33;

        f.write_binary( &clouds ).unwrap();

        assert! {
            DecryptResource::new(
                Resource::new( &f.path(), key() ).unwrap()
            ).is_err()
        }
    }

}
