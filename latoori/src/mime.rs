/// To avoid writing "pub const static str" over and over again.
macro_rules! mimes {
    ( $($name:ident : $value:expr)* ) => {
        $(
            #[ allow( unused ) ]
            pub const $name: &'static str = $value;
        )*
    };
}


mimes! {

    TEXT : "text/plain"
    HTML : "text/html; charset=utf-8"

    OCTET_STREAM : "application/octet-stream"

    JPEG : "image/jpeg"
    FAVICON : "image/x-icon"

}
