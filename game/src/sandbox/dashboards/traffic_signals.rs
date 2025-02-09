use std::collections::HashMap;

use abstutil::{prettyprint_usize, Counter, Timer};
use geom::{ArrowCap, Distance, Duration, Polygon, Time};
use map_gui::render::DrawOptions;
use map_gui::ID;
use map_model::{Intersection, IntersectionID, MovementID, PathStep, TurnType};
use sim::TripEndpoint;
use widgetry::{
    Color, DrawBaselayer, Drawable, EventCtx, GeomBatch, GfxCtx, HorizontalAlignment, Key, Line,
    Outcome, Panel, Spinner, State, Text, TextExt, VerticalAlignment, Widget,
};

use crate::app::{App, ShowEverything, Transition};
use crate::common::CommonState;
use crate::sandbox::dashboards::DashTab;

pub struct TrafficSignalDemand {
    panel: Panel,
    all_demand: HashMap<IntersectionID, Demand>,
    hour: Time,
    draw_all: Drawable,
    selected: Option<(Drawable, Text)>,
}

impl TrafficSignalDemand {
    pub fn new_state(ctx: &mut EventCtx, app: &mut App) -> Box<dyn State<App>> {
        let all_demand = ctx.loading_screen("predict all demand", |_, timer| {
            Demand::all_demand(app, timer)
        });

        app.primary.current_selection = None;
        assert!(app.primary.suspended_sim.is_none());
        app.primary.suspended_sim = Some(app.primary.clear_sim());

        let hour = Time::START_OF_DAY;
        let draw_all = Demand::draw_demand(ctx, app, &all_demand, hour);
        Box::new(TrafficSignalDemand {
            all_demand,
            hour,
            draw_all,
            selected: None,
            panel: Panel::new_builder(Widget::col(vec![
                DashTab::TrafficSignals.picker(ctx, app),
                Text::from_all(vec![
                    Line("Press "),
                    Key::LeftArrow.txt(ctx),
                    Line(" and "),
                    Key::RightArrow.txt(ctx),
                    Line(" to adjust the hour"),
                ])
                .into_widget(ctx),
                Widget::row(vec![
                    "Hour:".text_widget(ctx).centered_vert(),
                    Spinner::widget(
                        ctx,
                        "hour",
                        (Duration::ZERO, Duration::hours(24)),
                        Duration::hours(7),
                        Duration::hours(1),
                    ),
                ]),
            ]))
            .aligned(HorizontalAlignment::Center, VerticalAlignment::Top)
            .build(ctx),
        })
    }
}

impl State<App> for TrafficSignalDemand {
    fn event(&mut self, ctx: &mut EventCtx, app: &mut App) -> Transition {
        ctx.canvas_movement();
        // TODO Use MapspaceTooltips here?
        if ctx.redo_mouseover() {
            self.selected = None;
            app.recalculate_current_selection(ctx);
            if let Some(ID::Intersection(i)) = app.primary.current_selection.take() {
                let i = app.primary.map.get_i(i);
                if i.is_traffic_signal() {
                    // If we're mousing over something, the cursor is on the map.
                    let pt = ctx.canvas.get_cursor_in_map_space().unwrap();
                    for (arrow, count) in self.all_demand[&i.id].make_arrows(i, self.hour) {
                        if arrow.contains_pt(pt) {
                            let mut batch = GeomBatch::new();
                            batch.push(Color::hex("#EE702E"), arrow.clone());
                            if let Ok(p) = arrow.to_outline(Distance::meters(0.1)) {
                                batch.push(Color::WHITE, p);
                            }
                            let txt = Text::from(format!(
                                "{} / {}",
                                prettyprint_usize(count),
                                self.all_demand[&i.id].count(self.hour).sum()
                            ));
                            self.selected = Some((ctx.upload(batch), txt));
                            break;
                        }
                    }
                }
            }
        }

        let mut changed = false;
        match self.panel.event(ctx) {
            Outcome::Clicked(x) => match x.as_ref() {
                "close" => {
                    app.primary.sim = app.primary.suspended_sim.take().unwrap();
                    return Transition::Pop;
                }
                _ => unreachable!(),
            },
            Outcome::Changed(_) => {
                if let Some(tab) = DashTab::TrafficSignals.tab_changed(app, &self.panel) {
                    app.primary.sim = app.primary.suspended_sim.take().unwrap();
                    return Transition::Replace(tab.launch(ctx, app));
                }
                changed = true;
            }
            _ => {}
        }
        if ctx.input.pressed(Key::LeftArrow) {
            self.panel
                .modify_spinner(ctx, "hour", -1.0 * Duration::hours(1));
            changed = true;
        }
        if ctx.input.pressed(Key::RightArrow) {
            self.panel.modify_spinner(ctx, "hour", Duration::hours(1));
            changed = true;
        }
        if changed {
            self.hour = Time::START_OF_DAY + self.panel.spinner("hour");
            self.draw_all = Demand::draw_demand(ctx, app, &self.all_demand, self.hour);
        }

        Transition::Keep
    }

    fn draw_baselayer(&self) -> DrawBaselayer {
        DrawBaselayer::Custom
    }

    fn draw(&self, g: &mut GfxCtx, app: &App) {
        let mut opts = DrawOptions::new();
        opts.suppress_traffic_signal_details
            .extend(self.all_demand.keys().cloned());
        app.draw(g, opts, &ShowEverything::new());

        g.redraw(&self.draw_all);
        if let Some((ref draw, ref count)) = self.selected {
            g.redraw(draw);
            g.draw_mouse_tooltip(count.clone());
        }

        self.panel.draw(g);
        CommonState::draw_osd(g, app);
    }
}

struct Demand {
    // Unsorted
    raw: Vec<(Time, MovementID)>,
}

impl Demand {
    fn all_demand(app: &App, timer: &mut Timer) -> HashMap<IntersectionID, Demand> {
        let map = &app.primary.map;

        let mut all_demand = HashMap::new();
        for i in map.all_intersections() {
            if i.is_traffic_signal() {
                all_demand.insert(i.id, Demand { raw: Vec::new() });
            }
        }

        let paths = timer
            .parallelize(
                "predict routes",
                app.primary.sim.all_trip_info(),
                |(_, trip)| {
                    let departure = trip.departure;
                    TripEndpoint::path_req(trip.start, trip.end, trip.mode, map)
                        .and_then(|req| map.pathfind(req).ok())
                        .map(|path| (departure, path))
                },
            )
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        timer.start_iter("compute demand", paths.len());
        for (now, path) in paths {
            timer.next();
            // TODO For every step, increase 'now' by the best-case time to cross that step.
            for step in path.get_steps() {
                match step {
                    PathStep::Lane(_) | PathStep::ContraflowLane(_) => {}
                    PathStep::Turn(t) => {
                        if map.get_t(*t).turn_type == TurnType::SharedSidewalkCorner {
                            continue;
                        }
                        if let Some(demand) = all_demand.get_mut(&t.parent) {
                            demand
                                .raw
                                .push((now, map.get_i(t.parent).turn_to_movement(*t).0));
                        }
                    }
                }
            }
        }

        all_demand
    }

    fn count(&self, start: Time) -> Counter<MovementID> {
        let end = start + Duration::hours(1);
        let mut cnt = Counter::new();
        for (t, m) in &self.raw {
            if *t >= start && *t <= end {
                cnt.inc(*m);
            }
        }
        cnt
    }

    fn make_arrows(&self, i: &Intersection, hour: Time) -> Vec<(Polygon, usize)> {
        let cnt = self.count(hour);
        let total_demand = cnt.sum() as f64;

        let mut arrows = Vec::new();
        for (m, demand) in cnt.consume() {
            let percent = (demand as f64) / total_demand;
            let arrow = i.movements[&m]
                .geom
                .make_arrow(percent * Distance::meters(3.0), ArrowCap::Triangle);
            arrows.push((arrow, demand));
        }
        arrows
    }

    fn draw_demand(
        ctx: &mut EventCtx,
        app: &App,
        all_demand: &HashMap<IntersectionID, Demand>,
        hour: Time,
    ) -> Drawable {
        let mut batch = GeomBatch::new();
        for (i, demand) in all_demand {
            let mut outlines = Vec::new();
            for (arrow, _) in demand.make_arrows(app.primary.map.get_i(*i), hour) {
                if let Ok(p) = arrow.to_outline(Distance::meters(0.1)) {
                    outlines.push(p);
                }
                batch.push(Color::hex("#A3A3A3"), arrow);
            }
            batch.extend(Color::WHITE, outlines);
        }
        ctx.upload(batch)
    }
}
