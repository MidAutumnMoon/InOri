use ratatui::prelude::*;

pub struct Trunk;

pub struct Flower;

pub struct Leaf;

pub struct Wisteria;

impl Widget for Wisteria {
    fn render( self, area: Rect, buf: &mut Buffer )
    where
        Self: Sized
    {
        todo!()
    }
}
