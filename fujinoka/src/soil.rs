use ratatui::prelude::*;

#[derive(Default)]
pub struct Soil {}


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
