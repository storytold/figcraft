//! Preflight for vector exports, including subtrees rendered into embedded images.

use figcraft_doc::{Document, NodeId};

/// Maximum descendant depth for SVG/PDF export; each selected root is at depth zero.
pub const MAX_EXPORT_DEPTH: usize = 256;

/// Exporting this visible subtree would silently omit content at the traversal limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportDepthError {
    pub root: NodeId,
    pub node: NodeId,
}

impl std::fmt::Display for ExportDepthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot export layer {}: layer {} exceeds the maximum export depth of {MAX_EXPORT_DEPTH}; select an inner layer or reduce nesting",
            self.root, self.node
        )
    }
}

impl std::error::Error for ExportDepthError {}

/// Refuse a visible descendant deeper than [`MAX_EXPORT_DEPTH`] before computing bounds or
/// drawing. Hidden subtrees are not exported. Masks, boolean operands and rasterised effects
/// still depend on their descendants, so they participate in the same preflight.
///
/// The walk uses one iterator per ancestor (bounded auxiliary memory), rather than recursion or
/// collecting a whole subtree. A cycle also reaches the depth limit and is refused.
pub fn check_export_depth(doc: &Document, roots: &[NodeId]) -> Result<(), ExportDepthError> {
    for root in roots {
        let mut stack = vec![(std::slice::from_ref(root).iter(), 0)];
        while let Some((siblings, depth)) = stack.last_mut() {
            let Some(&id) = siblings.next() else {
                stack.pop();
                continue;
            };
            let depth = *depth;
            let Some(node) = doc.node(id) else { continue };
            if !node.visible {
                continue;
            }
            if depth > MAX_EXPORT_DEPTH {
                return Err(ExportDepthError { root: *root, node: id });
            }
            if !node.children.is_empty() {
                stack.push((node.children.iter(), depth + 1));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::NodeKind;

    #[test]
    fn export_depth_preflight_refuses_a_cyclic_subtree() {
        let mut doc = Document::new("cycle");
        let root = doc.create(doc.first_page().unwrap(), NodeKind::Group, "root", 0.0, 0.0, 1.0, 1.0).unwrap();
        doc.get_mut(root).unwrap().children.push(root);
        assert_eq!(check_export_depth(&doc, &[root]), Err(ExportDepthError { root, node: root }));
    }
}
