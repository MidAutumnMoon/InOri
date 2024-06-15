use std::path::{
    Path,
    PathBuf,
};


#[ derive( Debug ) ]
pub struct Location {
    resource_dirs: Vec<PathBuf>,
    system_json: Option<PathBuf>,
}


/// Which version of RPG Maker this game
/// is builtin with.
#[ derive( Debug ) ]
pub enum Layout {
    MV( Location ),
    MZ( Location ),
}

impl Layout {
    fn probe( toplevel: &Path )
        -> anyhow::Result<Self>
    {
        todo!()
    }

    pub fn location( &self ) -> &Location {
        match self {
            Self::MV( l ) | Self::MZ( l ) => l
        }
    }
}


// The game that the app is currently working on.
#[ derive( Debug ) ]
pub struct Game {
    toplevel: PathBuf,
    layout: Layout,
}

impl Game {
    pub fn new( toplevel: &Path )
        -> anyhow::Result<Self>
    {
        todo!()
    }
}
