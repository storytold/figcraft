//! Stable identity and bounded discovery for rendered video paints.

use std::collections::HashSet;
use std::sync::Arc;

use figcraft_color::{Paint, PaintKind, VideoPaint};
use figcraft_doc::{Document, Node, NodeId, NodeKind};
use tiny_skia::Pixmap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PaintDomain {
    Fill,
    Stroke,
    TextStyle(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaintId {
    pub node: NodeId,
    pub domain: PaintDomain,
    pub index: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VideoPatternPath {
    consumers: [Option<PaintId>; 2],
}

impl VideoPatternPath {
    pub fn pushed(mut self, consumer: PaintId) -> Option<Self> {
        let slot = self.consumers.iter_mut().find(|slot| slot.is_none())?;
        *slot = Some(consumer);
        Some(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VideoOccurrenceId {
    pub paint: PaintId,
    pub patterns: VideoPatternPath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VideoScopeId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VideoInstanceId {
    pub scope: VideoScopeId,
    pub occurrence: VideoOccurrenceId,
}

pub type VideoFrames = std::collections::HashMap<VideoInstanceId, Arc<Pixmap>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoRenderContext {
    pub scope: VideoScopeId,
    pub patterns: VideoPatternPath,
}

impl VideoRenderContext {
    pub fn paint(self, paint: PaintId) -> VideoPaintContext {
        VideoPaintContext { scope: self.scope, occurrence: VideoOccurrenceId { paint, patterns: self.patterns } }
    }

    pub fn pushed(self, consumer: PaintId) -> Option<Self> {
        Some(Self { scope: self.scope, patterns: self.patterns.pushed(consumer)? })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoPaintContext {
    pub scope: VideoScopeId,
    pub occurrence: VideoOccurrenceId,
}

impl VideoPaintContext {
    pub fn instance(self) -> VideoInstanceId {
        VideoInstanceId { scope: self.scope, occurrence: self.occurrence }
    }

    pub fn pattern_context(self) -> Option<VideoRenderContext> {
        Some(VideoRenderContext { scope: self.scope, patterns: self.occurrence.patterns.pushed(self.occurrence.paint)? })
    }
}

/// Visible paint batch from the node's actual rendering layout.
pub fn visit_paint_batches(node: &Node, mut visit: impl FnMut(PaintDomain, &[Paint])) {
    if matches!(&node.kind, NodeKind::Text(_)) {
        let Some((layout, _)) = figcraft_text::layout_node(node) else { return };
        let mut styles = Vec::new();
        for glyph in &layout.glyphs {
            if !styles.contains(&glyph.style) {
                styles.push(glyph.style);
            }
        }
        for decoration in &layout.decorations {
            if !styles.contains(&decoration.style) {
                styles.push(decoration.style);
            }
        }
        styles.sort_unstable();
        for style in styles {
            let fills = layout.styles.get(style).and_then(|s| s.fills.as_ref());
            match fills {
                Some(fills) => visit(PaintDomain::TextStyle(style), fills),
                None => visit(PaintDomain::Fill, &node.fills),
            }
        }
        if node.has_visible_stroke() {
            visit(PaintDomain::Stroke, &node.strokes);
        }
        return;
    }

    visit(PaintDomain::Fill, &node.fills);
    if node.has_visible_stroke() {
        visit(PaintDomain::Stroke, &node.strokes);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VideoPaintOccurrence<'a> {
    pub id: VideoOccurrenceId,
    pub paint: &'a VideoPaint,
}

/// Shared traversal limits; reuse one budget across roots and presentation scopes.
#[derive(Debug, Default)]
pub struct VideoScanBudget {
    visited_candidates: usize,
    active_occurrences: usize,
    source_hashes: HashSet<String>,
    truncated: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoScanReport {
    pub visited_candidates: usize,
    pub active_occurrences: usize,
    pub unique_source_hashes: usize,
    pub truncated: bool,
}

impl VideoScanBudget {
    pub fn report(&self) -> VideoScanReport {
        VideoScanReport {
            visited_candidates: self.visited_candidates,
            active_occurrences: self.active_occurrences,
            unique_source_hashes: self.source_hashes.len(),
            truncated: self.truncated,
        }
    }

    fn candidate(&mut self) -> bool {
        if self.visited_candidates == 4096 {
            self.truncated = true;
            return false;
        }
        self.visited_candidates += 1;
        true
    }
}

#[derive(Clone, Copy)]
struct Visit {
    node: NodeId,
    patterns: VideoPatternPath,
}

/// Visit effective video paints in deterministic node and paint order.
pub fn visit_video_occurrences(
    doc: &Document,
    root: NodeId,
    budget: &mut VideoScanBudget,
    mut visit: impl for<'a> FnMut(VideoPaintOccurrence<'a>),
) -> VideoScanReport {
    let mut stack = vec![Visit { node: root, patterns: VideoPatternPath::default() }];
    let mut visited_nodes = HashSet::new();
    let mut seen_occurrences = HashSet::new();

    while let Some(current) = stack.pop() {
        if !visited_nodes.insert((current.node, current.patterns)) {
            continue;
        }
        if !budget.candidate() {
            break;
        }
        let Some(node) = doc.node(current.node) else { continue };
        if !node.visible || node.opacity <= 0.0 || node.is_mask {
            continue;
        }

        let mut patterned_sources = Vec::new();
        visit_paint_batches(node, |domain, paints| {
            for (index, paint) in paints.iter().enumerate() {
                if !paint.visible || paint.opacity <= 0.0 {
                    continue;
                }
                let id = PaintId { node: node.id, domain, index };
                match &paint.kind {
                    PaintKind::Video(video) => {
                        let occurrence = VideoOccurrenceId { paint: id, patterns: current.patterns };
                        if !seen_occurrences.insert(occurrence) {
                            continue;
                        }
                        if budget.active_occurrences >= 16 {
                            budget.truncated = true;
                            continue;
                        }
                        if !budget.source_hashes.contains(&video.video) && budget.source_hashes.len() >= 4 {
                            budget.truncated = true;
                            continue;
                        }
                        budget.source_hashes.insert(video.video.clone());
                        budget.active_occurrences += 1;
                        visit(VideoPaintOccurrence { id: occurrence, paint: video });
                    }
                    PaintKind::Pattern(pattern) => {
                        if let Some(path) = current.patterns.pushed(id) {
                            let source = NodeId(pattern.source);
                            if doc.node(source).is_some_and(|source| !source.is_page()) {
                                patterned_sources.push(Visit { node: source, patterns: path });
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
        if let Some(node) = doc.node(current.node)
            && !matches!(node.kind, NodeKind::BooleanOp { .. })
            && !node.hides_contents()
        {
            for child in node.paint_order().rev() {
                stack.push(Visit { node: child, patterns: current.patterns });
            }
        }
        for source in patterned_sources.into_iter().rev() {
            stack.push(source);
        }
    }
    budget.report()
}
