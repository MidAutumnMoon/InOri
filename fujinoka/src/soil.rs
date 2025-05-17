use ratatui::prelude::*;

pub struct Soil {}

impl Default for Soil {
    fn default() -> Self {
        Self {}
    }
}

pub struct SoilWidget;

impl StatefulWidget for SoilWidget {
    type State = Soil;
    fn render( self, area: Rect, buf: &mut Buffer, state: &mut Self::State ) {
        Text::from( "Soil" )
            .yellow()
            .centered()
            .render(area, buf);
    }
}
