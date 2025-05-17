use ino_result::ResultExt;

fn main() {

    // ino_tracing::init_tracing_subscriber();

    fujinoka::Planet::new()
        .print_error_exit_process()
        .run()
        .print_error_exit_process();

}

// impl App {
//     fn new() -> anyhow::Result<Self> {
//         let exit_signal = AtomicBool::new( false ).pipe( Arc::new );
//         ctrlc::try_set_handler( {
//             let it = Arc::clone( &exit_signal );
//             move || { it.store( true, Ordering::SeqCst ); }
//         } )?;
//
//         let mut terminal = ratatui::try_init()
//             .context( "Failed to init ratatui terminal" )?;
//         terminal.hide_cursor()?;
//
//         // TODO: diable raw mode so that ctrl-c works, this needs to
//         // have a better event handling solution
//         ratatui::crossterm::terminal::disable_raw_mode()?;
//
//         Self {
//             cliopts: CliOpts {},
//             exit_signal: Arc::clone( &exit_signal ),
//             terminal,
//         }.pipe( Ok )
//     }
//
//     fn run( mut self ) -> anyhow::Result<()> {
//         use std::time::Duration;
//
//         let interval = Duration::from_millis( 1000 / TARGET_FRAMERATE as u64 );
//
//         let scene = scene::Scene::new();
//
//         while !self.should_exit() {
//             self.terminal.draw( |frame| scene.draw( frame ) )?;
//             std::thread::sleep( interval );
//         }
//
//         ratatui::try_restore()?;
//
//         Ok(())
//     }
//
//     #[ allow( clippy::unused_self ) ]
//     #[ inline ]
//     fn should_exit( &self ) -> bool {
//         self.exit_signal.load( Ordering::SeqCst )
//     }
// }
