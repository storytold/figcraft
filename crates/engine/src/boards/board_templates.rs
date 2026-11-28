//! Board templates: ready-made layouts (a retrospective, a brainstorm, a weekly planner, a flow
//! chart, a kanban board) placed on a board as ordinary board objects in one undo step. They are
//! built in code by running the board commands, so everything in them edits like anything else on
//! the board; their wording is FigCraft's own.

use std::sync::Arc;

use figcraft_doc::{Document, NodeId};
use serde_json::{Value, json};

use crate::board::require;
use crate::cmd::CommandSpec;
use crate::params::{id_of, num, string};
use crate::{EngineError, Result, Session, bad, spec};

/// A board template.
pub struct BoardTemplate {
    pub id: &'static str,
    pub name: &'static str,
    /// One line about what is in it.
    pub description: &'static str,
    /// Its width and height on the board.
    pub size: (f64, f64),
    make: fn(&mut Builder) -> Result<()>,
}

pub static TEMPLATES: &[BoardTemplate] = &[
    BoardTemplate {
        id: "retrospective",
        name: "Retrospective",
        description: "Three sections to look back on a stretch of work: what went well, what to improve, what to do next",
        size: (1280.0, 592.0),
        make: retrospective,
    },
    BoardTemplate {
        id: "brainstorm",
        name: "Brainstorm",
        description: "A question in the middle and four clusters of ideas around it",
        size: (1344.0, 760.0),
        make: brainstorm,
    },
    BoardTemplate {
        id: "weeklyPlanner",
        name: "Weekly planner",
        description: "A column for each working day with a note to start from",
        size: (1380.0, 632.0),
        make: weekly_planner,
    },
    BoardTemplate {
        id: "flowChart",
        name: "Flow chart",
        description: "Steps, a decision and its two outcomes joined by connectors",
        size: (1220.0, 360.0),
        make: flow_chart,
    },
    BoardTemplate {
        id: "kanban",
        name: "Kanban board",
        description: "To do, Doing and Done columns with cards to move along",
        size: (900.0, 812.0),
        make: kanban,
    },
];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "board.template",
            "Board template",
            "{name (a template id from board.templates), x?, y? (its top-left on the page; default centred in the view)} → {name, ids}  its sections, stickies, shapes, connectors and text, selected",
            true,
            template_cmd
        ),
        spec!("board.templates", "List board templates", "{} → [{id, name, description, width, height}]", false, |_, _| {
            Ok(Value::Array(
                TEMPLATES
                    .iter()
                    .map(|t| json!({"id": t.id, "name": t.name, "description": t.description, "width": t.size.0, "height": t.size.1}))
                    .collect(),
            ))
        }),
    ]
}

/// The template called `name` (its id, or its name in any case).
pub fn find(name: &str) -> Option<&'static BoardTemplate> {
    let name = name.trim();
    TEMPLATES.iter().find(|t| t.id.eq_ignore_ascii_case(name) || t.name.eq_ignore_ascii_case(name))
}

fn template_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.template";
    require(s, cmd)?;
    let names = || TEMPLATES.iter().map(|t| t.id).collect::<Vec<_>>().join(", ");
    let name = string(p, "name").ok_or_else(|| bad(cmd, format!("`name` is one of {}", names())))?;
    let t = find(&name).ok_or_else(|| bad(cmd, format!("no template `{name}`: one of {}", names())))?;
    let view = &s.ds()?.view;
    let at = (num(p, "x").unwrap_or(view.cx - t.size.0 / 2.0), num(p, "y").unwrap_or(view.cy - t.size.1 / 2.0));
    if !(at.0.is_finite() && at.1.is_finite()) {
        return Err(bad(cmd, "`x` and `y` are finite numbers"));
    }
    let page = s.page()?;
    let mut b = Builder { s, page, at, made: Vec::new() };
    (t.make)(&mut b)?;
    let made = b.made;
    s.set_selection(made.clone());
    Ok(json!({"name": t.id, "ids": made}))
}

/// A board with template `id` at the page's origin and nothing else (the picker's previews).
pub fn preview(id: &str) -> Result<Document> {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({"name": id}))?;
    s.execute("board.template", &json!({"name": id, "x": 0, "y": 0}))?;
    let ds = s.docs.pop().ok_or(EngineError::NoDocument)?;
    Ok(Arc::try_unwrap(ds.doc).unwrap_or_else(|d| (*d).clone()))
}

/// Places a template's objects: positions are from its top-left (`at` on the page), or inside the
/// section they go in.
struct Builder<'a> {
    s: &'a mut Session,
    page: NodeId,
    at: (f64, f64),
    /// What it placed on the page.
    made: Vec<NodeId>,
}

impl Builder<'_> {
    /// Run board command `cmd` (in this command's undo step); the layer it made.
    fn run(&mut self, cmd: &str, p: Value) -> Result<NodeId> {
        let spec = crate::cmd::find_command(cmd).ok_or_else(|| EngineError::UnknownCommand(cmd.into()))?;
        let v = (spec.run)(self.s, &p)?;
        v.get("id").and_then(id_of).ok_or_else(|| EngineError::Other(format!("`{cmd}` made no layer")))
    }

    /// A layer on the page at (`x`, `y`) from the template's corner.
    fn top(&mut self, cmd: &str, x: f64, y: f64, p: Value) -> Result<NodeId> {
        let id = self.run(cmd, placed(p, self.page, (self.at.0 + x, self.at.1 + y)))?;
        self.made.push(id);
        Ok(id)
    }

    fn title(&mut self, text: &str) -> Result<NodeId> {
        self.top("node.create", 0.0, 0.0, json!({"type": "text", "text": text, "props": {"fontSize": 32, "fills": ["#1E1E1E"]}}))
    }

    fn section(&mut self, name: &str, (x, y, w, h): (f64, f64, f64, f64), fill: &str) -> Result<NodeId> {
        self.top("node.create", x, y, json!({"type": "section", "name": name, "width": w, "height": h, "props": {"fills": [fill]}}))
    }

    /// A sticky in `section` (or on the page) at (`x`, `y`) in it.
    fn sticky(&mut self, section: Option<NodeId>, (x, y): (f64, f64), text: &str, color: &str) -> Result<NodeId> {
        let p = json!({"text": text, "color": color, "author": false});
        match section {
            Some(sec) => self.run("board.sticky", placed(p, sec, (x, y))),
            None => self.top("board.sticky", x, y, p),
        }
    }

    fn shape(&mut self, shape: &str, (x, y, w, h): (f64, f64, f64, f64), text: &str, color: &str) -> Result<NodeId> {
        self.top("board.shape", x, y, json!({"shape": shape, "width": w, "height": h, "text": text, "color": color}))
    }

    fn connector(&mut self, from: NodeId, to: NodeId, extra: Value) -> Result<NodeId> {
        let id = self.run("board.connector", with(extra, [("from", json!(from)), ("to", json!(to))]))?;
        self.made.push(id);
        Ok(id)
    }
}

/// Params `p` with `entries` added (`p` an object).
fn with<const N: usize>(mut p: Value, entries: [(&str, Value); N]) -> Value {
    if let Some(o) = p.as_object_mut() {
        o.extend(entries.map(|(k, v)| (k.to_string(), v)));
    }
    p
}

/// Params `p` placing a layer in `parent` at `at`.
fn placed(p: Value, parent: NodeId, at: (f64, f64)) -> Value {
    with(p, [("parent", json!(parent)), ("x", json!(at.0)), ("y", json!(at.1))])
}

fn retrospective(b: &mut Builder) -> Result<()> {
    b.title("Retrospective")?;
    let parts = [
        ("Went well", "#EAF6E1", "green", ["Something that helped us", "A win worth repeating"]),
        ("To improve", "#FCE8EF", "pink", ["Something that slowed us down", "A question we still have"]),
        ("Next steps", "#E6F0FC", "blue", ["One change to try next", "Who looks after it?"]),
    ];
    for (i, (name, fill, color, notes)) in parts.into_iter().enumerate() {
        let sec = b.section(name, (i as f64 * 440.0, 72.0, 400.0, 520.0), fill)?;
        for (j, note) in notes.into_iter().enumerate() {
            b.sticky(Some(sec), (100.0, 60.0 + j as f64 * 230.0), note, color)?;
        }
    }
    Ok(())
}

fn brainstorm(b: &mut Builder) -> Result<()> {
    let topic = b.shape("ellipse", (532.0, 300.0, 280.0, 160.0), "What are we trying to solve?", "violet")?;
    let clusters = [((0.0, 0.0), "yellow", 1), ((920.0, 0.0), "orange", 0), ((0.0, 560.0), "green", 1), ((920.0, 560.0), "blue", 0)];
    for ((x, y), color, near) in clusters {
        let notes = [b.sticky(None, (x, y), "An idea", color)?, b.sticky(None, (x + 224.0, y), "Build on it", color)?];
        if let Some(n) = notes.get(near) {
            b.connector(topic, *n, json!({"style": "straight", "endCap": "none"}))?;
        }
    }
    Ok(())
}

fn weekly_planner(b: &mut Builder) -> Result<()> {
    b.title("This week")?;
    let days = [
        ("Monday", "Plan the week"),
        ("Tuesday", "Focus for the day"),
        ("Wednesday", "Focus for the day"),
        ("Thursday", "Focus for the day"),
        ("Friday", "Wrap up and share"),
    ];
    for (i, (day, note)) in days.into_iter().enumerate() {
        let sec = b.section(day, (i as f64 * 280.0, 72.0, 260.0, 560.0), "#FFFFFF")?;
        b.sticky(Some(sec), (30.0, 60.0), note, if i % 2 == 0 { "yellow" } else { "orange" })?;
    }
    Ok(())
}

fn flow_chart(b: &mut Builder) -> Result<()> {
    let start = b.shape("ellipse", (0.0, 30.0, 160.0, 100.0), "Start", "green")?;
    let step = b.shape("rectangle", (240.0, 20.0, 180.0, 120.0), "Do the first step", "blue")?;
    let check = b.shape("diamond", (500.0, 0.0, 200.0, 160.0), "Did it work?", "yellow")?;
    let carry = b.shape("roundedRectangle", (800.0, 20.0, 180.0, 120.0), "Carry on", "blue")?;
    let done = b.shape("ellipse", (1060.0, 30.0, 160.0, 100.0), "Done", "green")?;
    let other = b.shape("roundedRectangle", (510.0, 240.0, 180.0, 120.0), "Try another way", "orange")?;
    let elbow = json!({"style": "elbow"});
    b.connector(start, step, elbow.clone())?;
    b.connector(step, check, elbow.clone())?;
    b.connector(check, carry, json!({"style": "elbow", "label": "Yes"}))?;
    b.connector(carry, done, elbow)?;
    b.connector(check, other, json!({"style": "elbow", "fromSide": "bottom", "toSide": "top", "label": "No"}))?;
    // Back round to the step: out of the side, into the step from below.
    b.connector(other, step, json!({"style": "elbow", "fromSide": "left", "toSide": "bottom"}))?;
    Ok(())
}

fn kanban(b: &mut Builder) -> Result<()> {
    b.title("Team board")?;
    let columns: [(&str, &str, &[&str]); 3] = [
        ("To do", "yellow", &["Write the brief", "Sketch the first screens", "Plan a test session"]),
        ("Doing", "blue", &["Review the copy"]),
        ("Done", "green", &["Set up the board"]),
    ];
    for (i, (name, color, cards)) in columns.into_iter().enumerate() {
        let sec = b.section(name, (i as f64 * 310.0, 72.0, 280.0, 740.0), "#FFFFFF")?;
        for (j, card) in cards.iter().enumerate() {
            b.sticky(Some(sec), (40.0, 60.0 + j as f64 * 220.0), card, color)?;
        }
    }
    Ok(())
}
