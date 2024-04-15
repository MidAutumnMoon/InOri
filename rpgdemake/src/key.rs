use crate::asset::ENCRYPTION_KEY_LEN;

use anyhow::{
    bail,
    ensure,
};

use tracing::debug;


/// The per-project key used to encrypt assets.
#[ derive( Debug ) ]
pub struct EncryptionKey {
    inner: Vec<u8>
}

impl EncryptionKey {
    #[ tracing::instrument ]
    pub fn parse_str( keystr: &str )
        -> anyhow::Result<Self>
    {
        use itertools::Itertools;

        debug!( "parse string into encryption key" );

        ensure! { keystr.len() == 2 * ENCRYPTION_KEY_LEN,
            "Encryption key \"{}\" doesn't match spec",
            keystr
        };

        let hex_chunks = keystr.chars().chunks( 2 );
        let mut key = Vec::with_capacity( ENCRYPTION_KEY_LEN );

        for chunk in hex_chunks.into_iter() {
            let c: Vec<u8> = chunk.map( |c| c as u8 ).collect();
            let c = hex::decode( c )?;
            key.extend( c )
        }

        debug!( ?key, "parsed key" );

        Ok( Self { inner: key } )
    }


    #[ tracing::instrument( skip_all ) ]
    pub fn parse_json( json: &str )
        -> anyhow::Result< Option<Self> >
    {
        use serde_json::{
            Value,
            from_str
        };

        debug!( "find encryptionKey in JSON" );

        let fields: Value = from_str( json )?;

        let key = match fields.get( "encryptionKey" ) {
            Some( v ) => match v {
                Value::String( s ) => s,
                _ => bail!( "Encryption key is not a string" )
            },
            None => return Ok( None ),
        };

        debug!( key, "found key" );

        Ok( Some (
            Self::parse_str( key )?
        ) )
    }


    pub fn get( &self ) -> &[u8] {
        self.inner.as_ref()
    }
}


#[ cfg( test ) ]
mod tests {

    const SYSTEM_JSON: &str =
        include_str!( "../fixture/System.json" );

    const EMPTY_SYSTEM_JSON: &str = "{}";

    const KEY_STR: &str = "bb145893824d809dcab45febae756d2b";

    const KEY_STR_INVALID: &str = "wow";

    const EXPECTED_KEY: &[u8] = &[
        187, 20,  88, 147, 130, 77,  128, 157,
        202, 180, 95, 235, 174, 117, 109, 43
    ];

    use super::*;


    #[ test ]
    fn str() {
        let key = EncryptionKey::parse_str( KEY_STR ).unwrap();
        assert_eq!( key.get(), EXPECTED_KEY );
    }

    #[ test ]
    fn str_invalid() {
        let key = EncryptionKey::parse_str( KEY_STR_INVALID );
        assert!( key.is_err() );
    }


    #[ test ]
    fn json() {
        let key = EncryptionKey::parse_json( SYSTEM_JSON )
            .unwrap();
        assert!( key.is_some() );
        assert_eq!( key.unwrap().get(), EXPECTED_KEY );
    }

    #[ test ]
    fn json_no_key() {
        let key = EncryptionKey::parse_json( EMPTY_SYSTEM_JSON )
            .unwrap();
        assert!( key.is_none() );
    }

}
