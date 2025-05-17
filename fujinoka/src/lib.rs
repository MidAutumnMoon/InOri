mod fuji;
mod soil;

use fuji::Wisteria;
use soil::Soil;

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

pub struct Model {
    last_frame: Instant,
    wisteria: Wisteria,
    soil: Soil,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            last_frame: Instant::now(),
            wisteria: Wisteria::default(),
            soil: Soil::default(),
        }
    }
}

pub struct Planet {
    model: Model,
    terminal: ratatui::DefaultTerminal,
    message: VecDeque<Message>,
}

#[ allow( clippy::missing_errors_doc ) ]
impl Planet {
    pub fn new() -> anyhow::Result<Self> {
        Self {
            model: Model::default(),
            terminal: ratatui::try_init()?,
            message: VecDeque::new(),
        }.pipe( Ok )
    }

    pub fn run( mut self ) -> anyhow::Result<()> {
        self.terminal.hide_cursor()?;
        loop {
            let now = Instant::now();
            let delta_time = now.duration_since( self.model.last_frame );
            let message = self.message
                .pop_front()
                .unwrap_or_default();
            match self.update( message, delta_time )? {
                PostUpdate::Quit => break,
                PostUpdate::Nothing => (),
            }
            // N.B. handle_event is a blocking function with timeout,
            // which means this won't be a busy loop even without sleep
            self.handle_event(
                TIME_QUOTA_PER_FRAME.saturating_sub( now.elapsed() )
            )?;
            self.model.last_frame = now;
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
        use fuji::WisteriaWidget;
        use soil::SoilWidget;

        self.terminal.draw( |frame| {
            use Constraint::Percentage;
            let [ wisteria_area, soil_area ] = Layout::default()
                .direction( Direction::Vertical )
                .constraints( [ Percentage( 95 ), Percentage( 5 ) ] )
                .areas( frame.area() );
            // Kind of a workaround of the fact `render_widget` takes
            // ownership of the widget.
            frame.render_stateful_widget(
                WisteriaWidget,
                wisteria_area,
                &mut self.model.wisteria,
            );
            frame.render_stateful_widget(
                SoilWidget,
                soil_area,
                &mut self.model.soil
            );
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

