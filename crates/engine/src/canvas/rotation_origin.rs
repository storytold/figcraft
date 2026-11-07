//! The rotation origin (Alt+R, `transform.rotationOrigin`): the point the selection turns about,
//! its centre unless moved. A moved origin belongs to the selection it was moved for, follows the
//! layers as they move and turn, and is dropped when the selection changes; it isn't saved in the
//! file. The corner rotation handles, the rotation field, `transform.rotate` and the 90° and 180°
//! rotations turn about it.

use figcraft_doc::{Document, NodeId};
use figcraft_geom::Point;

use crate::cmd::edit::roots;
use crate::tools::{SNAP_PX, selection_box};
use crate::{DocState, EngineError, Result, Session};

/// How near (screen pixels) a press must be to grab the origin handle.
const GRAB_PX: f64 = 8.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RotationOrigin {
    /// The selection it belongs to.
    pub ids: Vec<NodeId>,
    /// Where it was moved, in the first selected layer's own coordinates (so it follows the
    /// layer); `None`: the centre.
    pub at: Option<Point>,
    /// The handle shows and can be dragged (Alt+R toggles it, Escape hides it).
    pub editing: bool,
}

/// The selection box's corners, edge midpoints and centre (page coordinates); the centre last.
fn box_points(doc: &Document, sel: &[NodeId]) -> Option<[Point; 9]> {
    let [a, b, c, d] = selection_box(doc, &roots(doc, sel))?;
    let mid = |p: Point, q: Point| p.midpoint(q);
    Some([a, b, c, d, mid(a, b), mid(b, c), mid(c, d), mid(d, a), mid(a, c)])
}

impl DocState {
    fn current_origin(&self) -> Option<&RotationOrigin> {
        self.rotation_origin.as_ref().filter(|o| !o.ids.is_empty() && o.ids == self.selection)
    }

    /// Where the selection turns about when its origin was moved (page coordinates).
    pub fn moved_origin(&self) -> Option<Point> {
        let o = self.current_origin()?;
        let p = self.doc.world_transform(*o.ids.first()?) * o.at?;
        (p.x.is_finite() && p.y.is_finite()).then_some(p)
    }

    /// The selection's rotation origin (page coordinates): where it was moved, else the centre of
    /// its box.
    pub fn origin_point(&self) -> Option<Point> {
        self.moved_origin().or_else(|| box_points(&self.doc, &self.selection).map(|b| b[8]))
    }

    /// The origin handle while it shows (page coordinates; not while editing text or points).
    pub fn origin_handle(&self) -> Option<Point> {
        self.current_origin().filter(|o| o.editing && self.text_edit.is_none() && self.vector_edit.is_none())?;
        self.origin_point()
    }

    /// Whether `p` (page coordinates) is on the origin handle at `zoom`.
    pub fn on_origin_handle(&self, p: Point, zoom: f64) -> bool {
        self.origin_handle().is_some_and(|o| (o - p).hypot() * zoom.max(1e-6) <= GRAB_PX)
    }

    /// The selection's origin state, started for it if it has none.
    fn origin_mut(&mut self, cmd: &str) -> Result<&mut RotationOrigin> {
        if self.selection.is_empty() {
            return Err(EngineError::Disabled(cmd.to_string(), "nothing is selected".into()));
        }
        self.revision += 1;
        let ids = &self.selection;
        let o = self.rotation_origin.get_or_insert_with(RotationOrigin::default);
        if o.ids != *ids {
            *o = RotationOrigin { ids: ids.clone(), ..RotationOrigin::default() };
        }
        Ok(o)
    }
}

impl Session {
    /// Show or hide the selection's origin handle (`on`: `None` toggles). Returns whether it shows.
    pub fn edit_rotation_origin(&mut self, on: Option<bool>) -> Result<bool> {
        let o = self.ds_mut()?.origin_mut("transform.rotationOrigin")?;
        o.editing = on.unwrap_or(!o.editing);
        Ok(o.editing)
    }

    /// Move the selection's origin to `p` (page coordinates); `None` puts it back at the centre.
    pub fn set_rotation_origin(&mut self, p: Option<Point>) -> Result<()> {
        let ds = self.ds_mut()?;
        let first = ds.selection.first().copied();
        let world = first.map(|i| ds.doc.world_transform(i));
        let o = ds.origin_mut("transform.setRotationOrigin")?;
        o.at = match (p, world) {
            (Some(p), Some(w)) if w.determinant().abs() > 1e-12 => Some(w.inverse() * p).filter(|q| q.x.is_finite() && q.y.is_finite()),
            _ => None,
        };
        Ok(())
    }

    /// Hide the origin handle; false when it wasn't showing (Escape).
    pub(crate) fn end_origin_edit(&mut self) -> bool {
        let Ok(ds) = self.ds_mut() else { return false };
        let showing = ds.origin_handle().is_some();
        if let Some(o) = ds.rotation_origin.as_mut().filter(|_| showing) {
            o.editing = false;
            ds.revision += 1;
        }
        showing
    }

    /// Where a drag puts the origin: the pointer, or a corner, edge middle or the centre of the
    /// selection box within snapping distance (the centre: back to unmoved).
    pub(crate) fn drag_origin_to(&mut self, p: Point, zoom: f64) -> Result<()> {
        let ds = self.ds()?;
        let snapped = box_points(&ds.doc, &ds.selection).and_then(|b| {
            b.iter()
                .enumerate()
                .filter(|(_, q)| (**q - p).hypot() * zoom.max(1e-6) <= SNAP_PX)
                .min_by(|x, y| (*x.1 - p).hypot().total_cmp(&(*y.1 - p).hypot()))
                .map(|(i, q)| (i, *q))
        });
        match snapped {
            Some((8, _)) => self.set_rotation_origin(None),
            Some((_, q)) => self.set_rotation_origin(Some(q)),
            None => self.set_rotation_origin(Some(p)),
        }
    }
}
