use ratatui::prelude::*;

struct Trunk {}

struct Flower {}

struct Leaf {}

pub struct Wisteria {
    // trunk: Trunk,
    // flower: Flower,
    // leaf: Leaf,
}

impl Default for Wisteria {
    fn default() -> Self {
        Self {}
    }
}

pub struct WisteriaWidget;

impl StatefulWidget for WisteriaWidget {
    type State = Wisteria;
    fn render( self, area: Rect, buf: &mut Buffer, state: &mut Self::State ) {
        Text::from( "Hello" )
            .blue()
            .centered()
            .render( area, buf );
    }
}
