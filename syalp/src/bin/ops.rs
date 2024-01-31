use std::ops;


struct Awawa;


impl ops::Index<usize> for Awawa {
    type Output = usize;

    fn index( &self, index: usize )
        -> &'static Self::Output
    {
        Box::leak( Box::new( index+2 ) )
    }
}

fn main() {

    let a = Awawa;

    dbg!( a[1] );

}
