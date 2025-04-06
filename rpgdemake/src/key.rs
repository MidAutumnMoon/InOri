use anyhow::{
    bail,
    ensure,
};

use tracing::debug;


/// Length of encryption key.
/// Since the encryption method is a naive XOR,
/// the key length should equal to the length of the encrypted part.
pub const KEY_LEN: usize = crate::lore::ENCRYPTED_PART_LEN;

/// Text of hex of encryption key.
/// Each byte of key is stored as a hex ( 187 -> "bb" ),
/// so the total
pub const RAW_KEY_LEN: usize = 2 * KEY_LEN;


/// The per-project key used to encrypt assets.
#[ derive( Debug, Clone ) ]
pub struct Key {
    pub value: [ u8; KEY_LEN ],
}


impl TryFrom<&str> for Key {
    type Error = anyhow::Error;

    fn try_from( raw_key: &str ) -> anyhow::Result<Self> {
        debug!( "parse encryption key from str" );

        use itertools::Itertools;

        ensure! { raw_key.len() == RAW_KEY_LEN,
            "String \"{raw_key}\" is not a valid encryption key. \
            Maybe it's fake, obfuscated or broken.",
        };

        debug!( "decode hex values" );

        let key = raw_key.chars().chunks( 2 )
            .into_iter()
            .map( |ck| ck.map( |c| c as u8 ).collect_vec() )
            .map( hex::decode )
            .collect::< Result< Vec<_>, _ > >()?
            .into_iter().flatten().collect_vec()
        ;

        let value = match key.try_into() {
            Ok( v ) => v,
            Err( _ ) => anyhow::bail!( "Failed to convert key" )
        };

        Ok( Self { value } )
    }
}

impl Key {

    #[ tracing::instrument( skip_all ) ]
    pub fn parse_json( json: &str )
        -> anyhow::Result< Option<Self> >
    {
        use serde_json::{ Value, from_str };

        debug!( "try find encryption key in JSON" );

        let fields: Value = from_str( json )?;

        let key = match fields.get( "encryptionKey" ) {
            Some( v ) => match v {
                Value::String( s ) => s,
                _ => bail!{
                    "Found encryption key, \
                    but it can't be parsed into string"
                }
            },
            None => return Ok( None ),
        };

        debug!( key, "found key" );

        Ok( Some (
            Self::try_from( key.as_ref() )?
        ) )
    }

}


#[ cfg( test ) ]
mod tests {

    const JSON: &str =
        include_str!( "../tests/fixture/System.json" );

    const EMPTY_JSON: &str = "{}";

    const KEY_STR: &str = "bb145893824d809dcab45febae756d2b";

    const KEY_STR_INVALID: &str = "wow";

    const EXPECTED_KEY: &[u8] = &[
        187, 20,  88, 147, 130, 77,  128, 157,
        202, 180, 95, 235, 174, 117, 109, 43
    ];

    use super::*;


    #[ test ]
    fn str() {
        let key = Key::try_from( KEY_STR ).unwrap();
        assert_eq!( key.value, EXPECTED_KEY );
    }

    #[ test ]
    fn str_invalid() {
        let key = Key::try_from( KEY_STR_INVALID );
        assert!( key.is_err() );
    }


    #[ test ]
    fn json() {
        let key = Key::parse_json( JSON )
            .unwrap();
        assert!( key.is_some() );
        assert_eq!( key.unwrap().value, EXPECTED_KEY );
    }

    #[ test ]
    fn json_no_key() {
        let key = Key::parse_json( EMPTY_JSON )
            .unwrap();
        assert!( key.is_none() );
    }

}
