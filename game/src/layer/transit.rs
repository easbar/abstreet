use map_gui::tools::ColorDiscrete;
use map_model::{PathConstraints, PathStep};
use widgetry::{Drawable, EventCtx, GfxCtx, Outcome, Panel, Toggle, Widget};

use crate::app::App;
use crate::layer::{header, Layer, LayerOutcome, PANEL_PLACEMENT};

pub struct TransitNetwork {
    panel: Panel,
    unzoomed: Drawable,
    zoomed: Drawable,
}

impl Layer for TransitNetwork {
    fn name(&self) -> Option<&'static str> {
        Some("transit network")
    }
    fn event(&mut self, ctx: &mut EventCtx, app: &mut App) -> Option<LayerOutcome> {
        match self.panel.event(ctx) {
            Outcome::Clicked(x) => match x.as_ref() {
                "close" => {
                    return Some(LayerOutcome::Close);
                }
                _ => unreachable!(),
            },
            Outcome::Changed(_) => {
                *self = TransitNetwork::new(
                    ctx,
                    app,
                    self.panel.is_checked("show all routes"),
                    self.panel.is_checked("show buses"),
                    self.panel.is_checked("show trains"),
                );
            }
            _ => {}
        }
        None
    }
    fn draw(&self, g: &mut GfxCtx, app: &App) {
        self.panel.draw(g);
        if g.canvas.cam_zoom < app.opts.min_zoom_for_detail {
            g.redraw(&self.unzoomed);
        } else {
            g.redraw(&self.zoomed);
        }
    }
    fn draw_minimap(&self, g: &mut GfxCtx) {
        g.redraw(&self.unzoomed);
    }
}

impl TransitNetwork {
    pub fn new(
        ctx: &mut EventCtx,
        app: &App,
        show_all_routes: bool,
        show_buses: bool,
        show_trains: bool,
    ) -> TransitNetwork {
        let map = &app.primary.map;
        // TODO Same color for both?
        let mut categories = vec![
            ("bus lanes / rails", app.cs.bus_layer),
            ("transit stops", app.cs.bus_layer),
        ];
        if show_all_routes {
            categories.push(("routes", app.cs.bus_layer));
        }
        let mut colorer = ColorDiscrete::new(app, categories);
        for l in map.all_lanes() {
            if l.is_bus() && show_buses {
                colorer.add_l(l.id, "bus lanes / rails");
            }
            if l.is_light_rail() && show_trains {
                colorer.add_l(l.id, "bus lanes / rails");
            }
        }
        for bs in map.all_bus_stops().values() {
            if !bs.is_train_stop && show_buses {
                colorer.add_bs(bs.id, "transit stops");
            }
            if bs.is_train_stop && show_trains {
                colorer.add_bs(bs.id, "transit stops");
            }
        }
        if show_all_routes {
            for br in map.all_bus_routes() {
                if !show_buses && br.route_type == PathConstraints::Bus {
                    continue;
                }
                if !show_trains && br.route_type == PathConstraints::Train {
                    continue;
                }
                for req in br.all_steps(map) {
                    if let Ok(path) = map.pathfind(req) {
                        for step in path.get_steps() {
                            if let PathStep::Lane(l) = step {
                                colorer.add_l(*l, "routes");
                            }
                        }
                    }
                }
            }
        }
        let (unzoomed, zoomed, legend) = colorer.build(ctx);

        let panel = Panel::new_builder(Widget::col(vec![
            header(ctx, "Transit network"),
            Toggle::switch(ctx, "show all routes", None, show_all_routes),
            Toggle::switch(ctx, "show buses", None, show_buses),
            Toggle::switch(ctx, "show trains", None, show_trains),
            legend,
        ]))
        .aligned_pair(PANEL_PLACEMENT)
        .build(ctx);

        TransitNetwork {
            panel,
            unzoomed,
            zoomed,
        }
    }
}
