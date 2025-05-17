mod fuji;

use anyhow::Context;
use ratatui::prelude::*;
use tap::Pipe;

use std::collections::VecDeque;
use std::time::Instant;
use std::time::Duration;

pub const TARGET_FPS: u64 = 30;

pub const TIME_QUOTA_PER_FRAME: Duration =
    Duration::from_millis( 1000 / TARGET_FPS );

#[ derive( Default ) ]
pub enum Message {
    #[ default ]
    Render,
    Quit,
    // WindUp,
    // WindDown,
}

pub enum PostUpdate {
    Quit,
    Nothing,
}

pub struct State {
    last_frame: Instant,
    padding: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            last_frame: Instant::now(),
            padding: 0,
        }
    }
}

pub struct Planet {
    state: State,
    terminal: ratatui::DefaultTerminal,
    message: VecDeque<Message>,
}

#[ allow( clippy::missing_errors_doc ) ]
impl Planet {
    pub fn new() -> anyhow::Result<Self> {
        Self {
            state: State::default(),
            terminal: ratatui::try_init()?,
            message: VecDeque::new(),
        }.pipe( Ok )
    }

    pub fn run( mut self ) -> anyhow::Result<()> {
        self.terminal.hide_cursor()?;
        loop {
            let now = Instant::now();
            let delta_time = now.duration_since( self.state.last_frame );
            let message = self.message
                .pop_front()
                .unwrap_or_default();
            match self.update( message, delta_time )? {
                PostUpdate::Quit => break,
                PostUpdate::Nothing => (),
            }
            // N.B. handle_event is a blocking function with timeout,
            // which means this won't be a busy loop even without sleep
            self.handle_event( TIME_QUOTA_PER_FRAME - now.elapsed() )?;
            self.state.last_frame = now;
        };
        ratatui::try_restore()?;
        Ok(())
    }

    pub fn update( &mut self, message: Message, delta_time: Duration )
        -> anyhow::Result<PostUpdate>
    {
        match message {
            Message::Render => {
                self.view().context( "Failed to render view" )?;
            },
            Message::Quit => return Ok( PostUpdate::Quit ),
        }
        Ok( PostUpdate::Nothing )
    }

    pub fn view( &mut self ) -> anyhow::Result<()> {
        self.terminal.draw( |frame| {
            let text = format!( "{:->padding$}", "a", padding = self.state.padding );
            let a = Text::from( text ).blue().italic();
            frame.render_widget( a, frame.area() );
        } )?;
        Ok(())
    }

    fn handle_event( &mut self, timeout: Duration )
        -> anyhow::Result<()>
    {
        use ratatui::crossterm::event;
        use ratatui::crossterm::event::Event;
        if event::poll( timeout )? {
            match event::read()? {
                // TODO: currently it allows press any key to quit
                Event::Key(_) => {
                    self.message.push_back( Message::Quit );
                },
                _ => return Ok(()),
            }
        }
        Ok(())
    }
}

