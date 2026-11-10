//! Uno Platform C# Markup: a layer and its subtree as a fluent C# expression tree of WinUI
//! elements (`Uno.Extensions.Markup`), inside a small `UserControl` class.

use figcraft_doc::{Document, NodeId};

/// Uno C# Markup for layer `id` and its subtree.
pub(crate) fn uno(doc: &Document, id: NodeId) -> String {
    let name = doc.node(id).map_or("Layer", |n| n.name.as_str());
    format!("// {name}: Uno C# Markup is not mapped yet\n")
}
