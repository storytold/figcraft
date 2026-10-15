//! Pasting a Mermaid flowchart onto a board: its text becomes shapes with text and connectors,
//! laid out in layers.
//!
//! Reads the flowchart subset of the public Mermaid syntax: a `graph` or `flowchart` header with
//! a direction (TD, TB, BT, LR, RL), node shapes (`A[box]`, `A(rounded)`, `A([pill])`,
//! `A[[subroutine]]`, `A[(database)]`, `A((circle))`, `A{decision}`, `A{{hexagon}}`, `A[/data/]`,
//! `A[\\data\\]`, `A[/trapezoid\\]`, `A>flag]`), links (`-->`, `---`, `-.->`, `==>`, `--o`, `--x`)
//! with labels (`-->|yes|` or `-- yes -->`), chains (`A --> B --> C`) and `&` lists. Comments
//! (`%%`), styling (`style`, `classDef`, `class`, `click`, `linkStyle`) and `subgraph`/`end`
//! lines are skipped. Nothing beyond the text is read.

use std::collections::BTreeMap;

/// A node's shape as Mermaid draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Box,
    Rounded,
    Pill,
    Subroutine,
    Database,
    Circle,
    Decision,
    Hexagon,
    Data,
    DataLeft,
    Trapezoid,
    Flag,
}

impl Shape {
    /// The board shape it becomes: (`shape.setType`/`node.create` type, width, height).
    pub fn board(self) -> (&'static str, f64, f64) {
        match self {
            Shape::Box => ("rectangle", 160.0, 70.0),
            Shape::Rounded => ("roundedRectangle", 160.0, 70.0),
            Shape::Pill => ("roundedRectangle", 160.0, 56.0),
            Shape::Subroutine => ("predefinedProcess", 170.0, 70.0),
            Shape::Database => ("cylinder", 150.0, 90.0),
            Shape::Circle => ("ellipse", 110.0, 110.0),
            Shape::Decision => ("diamond", 180.0, 110.0),
            Shape::Hexagon => ("hexagon", 170.0, 80.0),
            Shape::Data => ("parallelogram", 170.0, 70.0),
            Shape::DataLeft => ("parallelogramLeft", 170.0, 70.0),
            Shape::Trapezoid => ("trapezoid", 170.0, 70.0),
            Shape::Flag => ("offPage", 160.0, 80.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub text: String,
    pub shape: Shape,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: String,
    pub dashed: bool,
    pub thick: bool,
    /// Ends in an arrow (`-->`), not a plain line (`---`).
    pub arrow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Down,
    Up,
    Right,
    Left,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Flowchart {
    pub direction: Direction,
    /// In the order they first appear.
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// The most nodes and links a pasted chart may have.
pub const MAX_NODES: usize = 500;
pub const MAX_EDGES: usize = 2000;

/// Whether `text` looks like a Mermaid flowchart (its first meaningful line is the header).
pub fn looks_like(text: &str) -> bool {
    text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("%%")).is_some_and(|l| {
        let w = l.split_whitespace().next().unwrap_or("");
        w.eq_ignore_ascii_case("graph") || w.eq_ignore_ascii_case("flowchart")
    })
}

/// Parse a flowchart (`None` when the text isn't one or holds no nodes).
pub fn parse(text: &str) -> Option<Flowchart> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("%%"));
    let header = lines.next()?;
    let mut words = header.split_whitespace();
    let kw = words.next()?;
    if !(kw.eq_ignore_ascii_case("graph") || kw.eq_ignore_ascii_case("flowchart")) {
        return None;
    }
    let direction = match words.next().map(|d| d.trim_end_matches(';').to_ascii_uppercase()).as_deref() {
        Some("LR") => Direction::Right,
        Some("RL") => Direction::Left,
        Some("BT") => Direction::Up,
        _ => Direction::Down,
    };
    let mut chart = Flowchart { direction, nodes: Vec::new(), edges: Vec::new() };
    // A header may carry the first statement after the direction (`graph LR; A-->B`).
    let rest: String = header.split_once(';').map(|(_, r)| r.to_string()).unwrap_or_default();
    for line in std::iter::once(rest.as_str()).chain(lines) {
        for stmt in line.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            statement(stmt, &mut chart);
            if chart.nodes.len() > MAX_NODES || chart.edges.len() > MAX_EDGES {
                return None;
            }
        }
    }
    (!chart.nodes.is_empty()).then_some(chart)
}

fn skip(stmt: &str) -> bool {
    let w = stmt.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    matches!(w.as_str(), "style" | "classdef" | "class" | "click" | "linkstyle" | "subgraph" | "end" | "direction")
}

/// One statement: a chain of node groups joined by links.
fn statement(stmt: &str, chart: &mut Flowchart) {
    if skip(stmt) {
        return;
    }
    let mut rest = stmt;
    let mut prev: Option<Vec<String>> = None;
    let mut pending: Option<Link> = None;
    while !rest.trim().is_empty() {
        let Some((ids, after)) = group(rest.trim_start(), chart) else { return };
        if let (Some(from), Some((label, dashed, thick, arrow))) = (&prev, pending.take()) {
            for f in from {
                for t in &ids {
                    if chart.edges.len() < MAX_EDGES {
                        chart.edges.push(Edge { from: f.clone(), to: t.clone(), label: label.clone(), dashed, thick, arrow });
                    }
                }
            }
        }
        prev = Some(ids);
        rest = after.trim_start();
        if rest.is_empty() {
            break;
        }
        let Some((link, after)) = link(rest) else { return };
        pending = Some(link);
        rest = after;
    }
}

/// `A & B[text] & C`: the node ids, and what follows.
fn group<'a>(s: &'a str, chart: &mut Flowchart) -> Option<(Vec<String>, &'a str)> {
    let mut ids = Vec::new();
    let mut rest = s;
    loop {
        let (id, after) = node(rest.trim_start(), chart)?;
        ids.push(id);
        let t = after.trim_start();
        match t.strip_prefix('&') {
            Some(more) => rest = more,
            None => return Some((ids, after)),
        }
    }
}

/// The shape delimiters, longest first: (open, close, shape).
const DELIMS: [(&str, &str, Shape); 12] = [
    ("([", "])", Shape::Pill),
    ("[[", "]]", Shape::Subroutine),
    ("[(", ")]", Shape::Database),
    ("((", "))", Shape::Circle),
    ("{{", "}}", Shape::Hexagon),
    ("[/", "/]", Shape::Data),
    ("[\\", "\\]", Shape::DataLeft),
    ("[/", "\\]", Shape::Trapezoid),
    ("[", "]", Shape::Box),
    ("(", ")", Shape::Rounded),
    ("{", "}", Shape::Decision),
    (">", "]", Shape::Flag),
];

/// One node: its id and optional shape with text; it's added (or its text and shape set) in the
/// chart. Returns the id and what follows.
fn node<'a>(s: &'a str, chart: &mut Flowchart) -> Option<(String, &'a str)> {
    let end = s.char_indices().find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '.')).map(|(i, _)| i).unwrap_or(s.len());
    let id = s.get(..end)?.to_string();
    if id.is_empty() {
        return None;
    }
    let mut rest = s.get(end..)?;
    let mut shaped = None;
    for (open, close, shape) in DELIMS {
        if let Some(inner) = rest.strip_prefix(open)
            && let Some(at) = inner.find(close)
        {
            // `[/x\]` and `[/x/]` share an opening: take the closing that comes first.
            if open == "[/" {
                let other = if close == "/]" { "\\]" } else { "/]" };
                if inner.find(other).is_some_and(|o| o < at) {
                    continue;
                }
            }
            let text = inner.get(..at).unwrap_or("").trim().trim_matches('"').to_string();
            shaped = Some((text, shape));
            rest = inner.get(at + close.len()..).unwrap_or("");
            break;
        }
    }
    match chart.nodes.iter_mut().find(|n| n.id == id) {
        Some(n) => {
            if let Some((text, shape)) = shaped {
                n.text = text;
                n.shape = shape;
            }
        }
        None => {
            let (text, shape) = shaped.unwrap_or_else(|| (id.clone(), Shape::Box));
            chart.nodes.push(Node { id: id.clone(), text, shape });
        }
    }
    Some((id, rest))
}

/// A link's label and look: (label, dashed, thick, arrow).
type Link = (String, bool, bool, bool);

/// A link and its label, and what follows.
fn link(s: &str) -> Option<(Link, &str)> {
    // The link's own characters (an `o` or `x` end only when a node doesn't start there).
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut end = 0;
    for (k, (i, c)) in chars.iter().enumerate() {
        let next_alnum = chars.get(k + 1).is_some_and(|(_, n)| n.is_alphanumeric());
        let ok = matches!(c, '-' | '.' | '=' | '>' | '<') || (matches!(c, 'o' | 'x') && k > 0 && !next_alnum);
        if !ok {
            break;
        }
        end = i + c.len_utf8();
    }
    let token = s.get(..end)?;
    if token.chars().count() < 2 {
        return None;
    }
    let mut tail = s.get(end..)?.trim_start();
    // `-- yes -->`: the words come between the link's opening and its end.
    if matches!(token, "--" | "==" | "-.") && !tail.starts_with('|') {
        let ends = ["-->", "---", "-.->", ".->", "==>", "==="];
        let (p, e) = ends.iter().filter_map(|e| tail.find(e).map(|p| (p, *e))).min_by_key(|(p, _)| *p)?;
        let label = tail.get(..p).unwrap_or("").trim().to_string();
        let (dashed, thick, arrow) = style_of(&format!("{token}{e}"));
        return Some(((label, dashed, thick, arrow), tail.get(p + e.len()..).unwrap_or("")));
    }
    // `-->|yes|`
    let mut label = String::new();
    if let Some(t) = tail.strip_prefix('|')
        && let Some(e) = t.find('|')
    {
        label = t.get(..e).unwrap_or("").trim().to_string();
        tail = t.get(e + 1..).unwrap_or("");
    }
    let (dashed, thick, arrow) = style_of(token);
    Some(((label, dashed, thick, arrow), tail))
}

fn style_of(token: &str) -> (bool, bool, bool) {
    (token.contains('.'), token.contains('='), token.ends_with('>'))
}

/// Where each node goes, its centre in a layered layout: each node one layer after the ones
/// leading to it (cycles broken), layers ordered to cross less, spaced by the nodes' sizes.
pub fn layout(chart: &Flowchart) -> BTreeMap<String, (f64, f64)> {
    let n = chart.nodes.len();
    let idx: BTreeMap<&str, usize> = chart.nodes.iter().enumerate().map(|(i, nd)| (nd.id.as_str(), i)).collect();
    let edges: Vec<(usize, usize)> =
        chart.edges.iter().filter_map(|e| Some((*idx.get(e.from.as_str())?, *idx.get(e.to.as_str())?))).filter(|(a, b)| a != b).collect();
    // Layers: longest path from the sources, over the edges that go "forward" in first-seen order
    // (back edges break the cycles).
    let mut layer = vec![0usize; n];
    for _ in 0..n {
        let mut changed = false;
        for (a, b) in &edges {
            if a < b && layer[*b] < layer[*a] + 1 {
                layer[*b] = layer[*a] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let depth = layer.iter().copied().max().unwrap_or(0) + 1;
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for i in 0..n {
        if let Some(r) = rows.get_mut(layer[i]) {
            r.push(i);
        }
    }
    // Order within layers by the mean position of their parents (a few sweeps).
    for _ in 0..4 {
        for li in 1..depth {
            let (above, below) = rows.split_at_mut(li);
            let Some(prev) = above.last() else { continue };
            let Some(row) = below.first_mut() else { continue };
            let pos: BTreeMap<usize, usize> = prev.iter().enumerate().map(|(p, i)| (*i, p)).collect();
            let key = |i: &usize| -> f64 {
                let ps: Vec<f64> = edges.iter().filter(|(_, b)| b == i).filter_map(|(a, _)| pos.get(a)).map(|p| *p as f64).collect();
                if ps.is_empty() { f64::MAX } else { ps.iter().sum::<f64>() / ps.len() as f64 }
            };
            row.sort_by(|a, b| key(a).total_cmp(&key(b)).then(a.cmp(b)));
        }
    }
    // Positions: layers along the flow, nodes across, each layer centred.
    let size = |i: usize| chart.nodes.get(i).map(|nd| nd.shape.board()).map(|(_, w, h)| (w, h)).unwrap_or((160.0, 70.0));
    let horizontal = matches!(chart.direction, Direction::Right | Direction::Left);
    let (gap_along, gap_across) = (90.0, 60.0);
    let mut out = BTreeMap::new();
    let mut along = 0.0;
    for row in &rows {
        let thick = row.iter().map(|i| if horizontal { size(*i).0 } else { size(*i).1 }).fold(0.0, f64::max);
        let widths: Vec<f64> = row.iter().map(|i| if horizontal { size(*i).1 } else { size(*i).0 }).collect();
        let total = widths.iter().sum::<f64>() + gap_across * widths.len().saturating_sub(1) as f64;
        let mut across = -total / 2.0;
        for (k, i) in row.iter().enumerate() {
            let w = widths.get(k).copied().unwrap_or(0.0);
            let (c_along, c_across) = (along + thick / 2.0, across + w / 2.0);
            let (x, y) = if horizontal { (c_along, c_across) } else { (c_across, c_along) };
            let (x, y) = match chart.direction {
                Direction::Up => (x, -y),
                Direction::Left => (-x, y),
                _ => (x, y),
            };
            if let Some(nd) = chart.nodes.get(*i) {
                out.insert(nd.id.clone(), (x, y));
            }
            across += w + gap_across;
        }
        along += thick + gap_along;
    }
    out
}
