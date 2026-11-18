//! An AI agent attached over MCP, shown like a person in a live session while it works: its avatar
//! in the people row and, on the canvas, an outline in its colour around the layers it last edited
//! or selected, with its name. Both go a short while after its last command
//! ([`figcraft_engine::agent`]), and only the file it works in shows them.

use egui::{Align2, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use figcraft_engine::agent::AgentPresence;
use web_time::Instant;

use crate::FigApp;
use crate::a11y::Tip;
use crate::theme::{AGENT, Tokens};

/// The agent at work in the file in front; asks for the frame where it goes.
pub(crate) fn shown<'a>(app: &'a FigApp, ctx: &egui::Context) -> Option<&'a AgentPresence> {
    let now = Instant::now();
    let p = app.session.agent_presence_at(now).filter(|p| app.session.ds().is_ok_and(|d| d.uid == p.doc))?;
    ctx.request_repaint_after(p.left(now));
    Some(p)
}

/// Its avatar at `c` in the people row: ringed in its colour while it works, its name as the tip.
pub(super) fn avatar(ui: &mut Ui, c: Pos2, r: f32, name: &str) {
    let t = Tokens::get(ui.ctx());
    ui.interact(Rect::from_center_size(c, vec2(2.0 * r, 2.0 * r)), ui.scope_id().with("live_agent"), Sense::hover()).tip(name);
    ui.painter().circle_stroke(c, r + 1.0, Stroke::new(2.0, t.panel));
    super::avatar(ui.painter(), c, r, name, AGENT);
    ui.painter().circle_stroke(c, r + 2.5, Stroke::new(2.0, AGENT));
}

/// Its layers on the page in front outlined in its colour, its name over them.
pub fn draw(app: &FigApp, painter: &egui::Painter, ctx: &egui::Context) {
    let Some(p) = shown(app, ctx) else { return };
    let Ok(ds) = app.session.ds() else { return };
    let (doc, page) = (&ds.doc, ds.page);
    let mut over: Option<Rect> = None;
    for q in p.layers.iter().filter(|n| doc.page_of(**n) == Some(page)).filter_map(|n| crate::canvas::node_quad(app, doc, *n)) {
        painter.add(Shape::closed_line(q.to_vec(), Stroke::new(1.5, AGENT)));
        let r = Rect::from_points(&q);
        over = Some(over.map_or(r, |o| o.union(r)));
    }
    if let Some(r) = over {
        super::name_tag(painter, pos2(r.min.x, r.min.y - 4.0), Align2::LEFT_BOTTOM, &p.name, AGENT);
    }
}

#[cfg(test)]
mod tests {
    use figcraft_engine::Session;
    use serde_json::json;

    use super::shown;
    use crate::present_tests::Ui;

    fn rect(u: &mut Ui, agent: Option<&str>) {
        let mut p = json!({"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 40, "height": 40}});
        if let Some(a) = agent {
            p["agent"] = json!(a);
        }
        let ctx = u.ctx.clone();
        crate::control::handle(&mut u.app, &ctx, "engine.execute", &p).unwrap();
        u.frames(2, Vec::new);
    }

    fn labels(u: &mut Ui) -> Vec<(String, egui::Pos2)> {
        let out = u.frame_output(vec![]);
        let nodes = out.platform_output.accesskit_update.map(|t| t.nodes).unwrap_or_default();
        nodes
            .into_iter()
            .filter_map(|(_, n)| {
                let b = n.bounds()?;
                Some((n.label()?.to_string(), egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)))
            })
            .collect()
    }

    #[test]
    fn the_people_menu_still_lists_you_beside_an_agent() {
        let mut u = Ui::new(Session::new());
        u.ctx.enable_accesskit();
        rect(&mut u, Some("Claude"));
        let (_, at) = labels(&mut u).into_iter().find(|(l, _)| l == "People in this file").expect("the people row's menu");
        u.button(at, true);
        u.button(at, false);
        u.frames(2, Vec::new);
        let shown = labels(&mut u);
        assert!(shown.iter().any(|(l, _)| l.ends_with("(you)")), "{shown:?}");
    }

    #[test]
    fn an_attached_agent_shows_in_the_file_it_works_in() {
        let mut u = Ui::new(Session::new());
        rect(&mut u, None);
        assert!(shown(&u.app, &u.ctx).is_none(), "the control channel's own calls are the user's");
        rect(&mut u, Some("Claude"));
        let p = shown(&u.app, &u.ctx).expect("the agent shows");
        assert_eq!((p.name.as_str(), p.layers.len()), ("Claude (agent)", 1));
        u.run("file.new", json!({}));
        assert!(shown(&u.app, &u.ctx).is_none(), "another file in front shows no agent");
    }
}
