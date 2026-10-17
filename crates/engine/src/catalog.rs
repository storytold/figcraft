//! The incumbent's main-menu tree with stable FigCraft command ids. A command registered under a
//! catalog id lights up its menu item, and `cargo xtask parity` measures coverage against it.
//!
//! Entries: (menu path, label, command id). `/` separates submenus; a label of `-` is a
//! separator. Shortcuts live in [`crate::keymap`].

pub struct MenuItem {
    pub menu: &'static str,
    pub label: &'static str,
    pub id: &'static str,
}

macro_rules! m {
    ($menu:expr, $label:expr, $id:expr) => {
        MenuItem { menu: $menu, label: $label, id: $id }
    };
}

pub const SEP: &str = "-";

pub static MENU: &[MenuItem] = &[
    // File
    m!("File", "New design file", "file.new"),
    // FigCraft's own: starter templates (not counted by `parity`, like the other items of its own
    // listed in `OWN`).
    m!("File", "New from template…", "file.newFromTemplate"),
    m!("File", "New window", "window.new"),
    m!("File", "Open…", "file.open"),
    m!("File", "Import from file…", "file.import"),
    m!("File", "Place image…", "file.placeImage"),
    m!("File", SEP, ""),
    m!("File", "Save", "file.save"),
    m!("File", "Save as…", "file.saveAs"),
    m!("File", "Save to version history…", "file.saveVersion"),
    m!("File", "Show version history", "file.versionHistory"),
    // FigCraft's own: branches of the file.
    m!("File", "Create branch…", "branch.create"),
    m!("File", SEP, ""),
    // FigCraft's own (live sessions): not in the reference menu, so not counted by `parity`.
    m!("File", "Join live session…", "collab.join"),
    m!("File", "Publish to server…", "collab.publish"),
    m!("File", SEP, ""),
    m!("File", "Export…", "file.export"),
    m!("File", "Export frames to PDF…", "file.exportFramesPdf"),
    m!("File", "Export selection as SVG…", "file.exportSvg"),
    m!("File", SEP, ""),
    m!("File", "Duplicate", "file.duplicate"),
    m!("File", "Rename", "file.rename"),
    m!("File", "Color profile…", "file.colorProfile"),
    m!("File", "Revert to saved", "file.revert"),
    m!("File", "Close tab", "file.close"),
    m!("File", "Open recent", "file.openRecent"),
    // Edit
    m!("Edit", "Undo", "edit.undo"),
    m!("Edit", "Redo", "edit.redo"),
    m!("Edit", SEP, ""),
    m!("Edit/Copy as", "Copy as text", "edit.copyAsText"),
    m!("Edit/Copy as", "Copy as CSS", "edit.copyAsCss"),
    m!("Edit/Copy as", "Copy as SVG", "edit.copyAsSvg"),
    m!("Edit/Copy as", "Copy as PNG", "edit.copyAsPng"),
    m!("Edit/Copy as", "Copy link to selection", "edit.copyLink"),
    m!("Edit", "Cut", "edit.cut"),
    m!("Edit", "Copy", "edit.copy"),
    m!("Edit", "Paste", "edit.paste"),
    m!("Edit", "Paste over selection", "edit.pasteOver"),
    m!("Edit", "Paste to replace", "edit.pasteReplace"),
    m!("Edit", "Duplicate", "edit.duplicate"),
    m!("Edit", "Delete", "edit.delete"),
    m!("Edit", SEP, ""),
    m!("Edit", "Find and replace…", "edit.findReplace"),
    m!("Edit", "Copy properties", "edit.copyProperties"),
    m!("Edit", "Paste properties", "edit.pasteProperties"),
    m!("Edit", "Pick color", "edit.pickColor"),
    m!("Edit", "Set default properties", "edit.setDefaultProperties"),
    m!("Edit", SEP, ""),
    m!("Edit", "Select all", "select.all"),
    m!("Edit", "Select matching layers", "select.matching"),
    m!("Edit", "Select none", "select.none"),
    m!("Edit", "Select inverse", "select.inverse"),
    m!("Edit/Select all with same", "Fill", "select.sameFill"),
    m!("Edit/Select all with same", "Stroke", "select.sameStroke"),
    m!("Edit/Select all with same", "Effect", "select.sameEffect"),
    m!("Edit/Select all with same", "Text properties", "select.sameText"),
    m!("Edit/Select all with same", "Font", "select.sameFont"),
    m!("Edit/Select all with same", "Instance", "select.sameInstance"),
    m!("Edit/Select all with same", "Properties", "select.sameProperties"),
    // View
    m!("View", "Pixel grid", "view.pixelGrid"),
    m!("View", "Layout grids", "view.layoutGrids"),
    m!("View", "Rulers", "view.rulers"),
    m!("View", "Slices", "view.slices"),
    m!("View", "Comments", "view.comments"),
    m!("View", "Multiplayer cursors", "view.multiplayerCursors"),
    m!("View", SEP, ""),
    m!("View/Outlines", "Show outlines", "view.outlines"),
    m!("View/Outlines", "Show outlines for selection", "view.outlinesSelection"),
    m!("View/Outlines", SEP, ""),
    m!("View/Outlines", "Include hidden layers", "view.outlinesHiddenLayers"),
    m!("View/Outlines", "Include object bounds", "view.outlinesObjectBounds"),
    m!("View", "Pixel preview", "view.pixelPreview"),
    m!("View", "Mask outlines", "view.maskOutlines"),
    m!("View", "Frame outlines", "view.frameOutlines"),
    // FigCraft's own: the canvas shows sRGB, so Display P3 files can mark what it can't show.
    m!("View", "Soft-proof sRGB", "view.proofSrgb"),
    m!("View", SEP, ""),
    m!("View/Panels", "Show/Hide UI", "view.toggleUi"),
    // FigCraft's own: the window fills the screen (F11), for setups that hide the taskbar then.
    m!("View", "Full screen", "view.fullscreen"),
    m!("View/Panels", "Reset panel widths", "view.panelWidths"),
    m!("View/Panels", "Layers", "view.layers"),
    m!("View/Panels", "Assets", "view.assets"),
    m!("View/Panels", "Design", "view.design"),
    m!("View/Panels", "Prototype", "view.prototype"),
    m!("View", SEP, ""),
    m!("View", "Zoom in", "view.zoomIn"),
    m!("View", "Zoom out", "view.zoomOut"),
    m!("View", "Zoom to 100%", "view.zoom100"),
    m!("View", "Zoom to 50%", "view.zoom50"),
    m!("View", "Zoom to 200%", "view.zoom200"),
    m!("View", "Zoom to fit", "view.zoomFit"),
    m!("View", "Zoom to selection", "view.zoomSelection"),
    m!("View", "Zoom to previous frame", "view.prevFrame"),
    m!("View", "Zoom to next frame", "view.nextFrame"),
    m!("View", SEP, ""),
    m!("View", "Previous page", "view.prevPage"),
    m!("View", "Next page", "view.nextPage"),
    m!("View/Theme", "Light", "view.themeLight"),
    m!("View/Theme", "Dark", "view.themeDark"),
    m!("View/Theme", "Follow system", "view.themeSystem"),
    // Object
    m!("Object", "Frame selection", "object.frameSelection"),
    m!("Object", "Group selection", "object.group"),
    m!("Object", "Ungroup selection", "object.ungroup"),
    m!("Object", "Remove frame", "object.removeFrame"),
    m!("Object", "Resize to fit", "object.resizeToFit"),
    m!("Object", SEP, ""),
    m!("Object", "Add auto layout", "layout.add"),
    m!("Object", "Remove auto layout", "layout.remove"),
    m!("Object", "Suggest auto layout", "layout.suggest"),
    m!("Object", SEP, ""),
    m!("Object", "Create component", "component.create"),
    m!("Object", "Create multiple components", "component.createMultiple"),
    m!("Object", "Combine as variants", "component.combineVariants"),
    m!("Object", "Reset all changes", "component.resetInstance"),
    m!("Object", "Detach instance", "component.detach"),
    m!("Object", "Go to main component", "component.goToMain"),
    m!("Object", "Push changes to main component", "component.pushChanges"),
    m!("Object", "Restore main component", "component.restoreMain"),
    m!("Object", SEP, ""),
    m!("Object", "Use as mask", "object.mask"),
    m!("Object", "Release mask", "object.releaseMask"),
    m!("Object", "Set as thumbnail", "object.setThumbnail"),
    m!("Object", SEP, ""),
    m!("Object", "Bring to front", "arrange.front"),
    m!("Object", "Bring forward", "arrange.forward"),
    m!("Object", "Send backward", "arrange.backward"),
    m!("Object", "Send to back", "arrange.back"),
    m!("Object", SEP, ""),
    m!("Object", "Flip horizontal", "object.flipH"),
    m!("Object", "Flip vertical", "object.flipV"),
    m!("Object", "Rotate 180°", "object.rotate180"),
    m!("Object", "Rotate 90° left", "object.rotateLeft"),
    m!("Object", "Rotate 90° right", "object.rotateRight"),
    m!("Object", SEP, ""),
    m!("Object", "Flatten", "vector.flatten"),
    m!("Object", "Outline stroke", "vector.outlineStroke"),
    m!("Object/Boolean groups", "Union", "bool.union"),
    m!("Object/Boolean groups", "Subtract", "bool.subtract"),
    m!("Object/Boolean groups", "Intersect", "bool.intersect"),
    m!("Object/Boolean groups", "Exclude", "bool.exclude"),
    m!("Object/Boolean groups", "Flatten", "bool.flatten"),
    m!("Object", "Rasterize selection", "object.rasterize"),
    m!("Object", SEP, ""),
    m!("Object", "Show/Hide selection", "object.toggleVisible"),
    m!("Object", "Lock/Unlock selection", "object.toggleLock"),
    m!("Object", "Hide other layers", "object.hideOthers"),
    m!("Object", "Collapse layers", "layers.collapseAll"),
    m!("Object", SEP, ""),
    m!("Object", "Remove fill", "object.removeFill"),
    m!("Object", "Remove stroke", "object.removeStroke"),
    m!("Object", "Swap fill and stroke", "object.swapFillStroke"),
    m!("Object", SEP, ""),
    m!("Object", "Rename", "object.rename"),
    m!("Object", "Batch rename…", "object.batchRename"),
    m!("Object", "Move to page", "object.moveToPage"),
    m!("Object/Select layer", "Select children", "select.children"),
    m!("Object/Select layer", "Select parent", "select.parent"),
    m!("Object/Select layer", "Select next sibling", "select.next"),
    m!("Object/Select layer", "Select previous sibling", "select.previous"),
    // Text
    m!("Text", "Bold", "text.bold"),
    m!("Text", "Italic", "text.italic"),
    m!("Text", "Underline", "text.underline"),
    m!("Text", "Strikethrough", "text.strikethrough"),
    m!("Text", "Create link", "text.link"),
    m!("Text", SEP, ""),
    m!("Text", "Bulleted list", "text.bulletList"),
    m!("Text", "Numbered list", "text.numberList"),
    m!("Text", SEP, ""),
    m!("Text/Alignment", "Text align left", "text.alignLeft"),
    m!("Text/Alignment", "Text align center", "text.alignCenter"),
    m!("Text/Alignment", "Text align right", "text.alignRight"),
    m!("Text/Alignment", "Text align justified", "text.alignJustify"),
    m!("Text/Adjust", "Increase font size", "text.sizeUp"),
    m!("Text/Adjust", "Decrease font size", "text.sizeDown"),
    m!("Text/Adjust", "Increase font weight", "text.weightUp"),
    m!("Text/Adjust", "Decrease font weight", "text.weightDown"),
    m!("Text/Adjust", "Increase line height", "text.lineHeightUp"),
    m!("Text/Adjust", "Decrease line height", "text.lineHeightDown"),
    m!("Text/Adjust", "Increase letter spacing", "text.trackingUp"),
    m!("Text/Adjust", "Decrease letter spacing", "text.trackingDown"),
    m!("Text/Case", "Original case", "text.caseOriginal"),
    m!("Text/Case", "Uppercase", "text.caseUpper"),
    m!("Text/Case", "Lowercase", "text.caseLower"),
    m!("Text/Case", "Title case", "text.caseTitle"),
    m!("Text", SEP, ""),
    m!("Text", "Missing fonts…", "text.missingFonts"),
    // Arrange
    m!("Arrange", "Align left", "align.left"),
    m!("Arrange", "Align horizontal centers", "align.hcenter"),
    m!("Arrange", "Align right", "align.right"),
    m!("Arrange", "Align top", "align.top"),
    m!("Arrange", "Align vertical centers", "align.vcenter"),
    m!("Arrange", "Align bottom", "align.bottom"),
    m!("Arrange", SEP, ""),
    m!("Arrange", "Distribute horizontal spacing", "distribute.horizontal"),
    m!("Arrange", "Distribute vertical spacing", "distribute.vertical"),
    m!("Arrange", "Tidy up", "arrange.tidy"),
    m!("Arrange", SEP, ""),
    m!("Arrange", "Pack horizontal", "arrange.packH"),
    m!("Arrange", "Pack vertical", "arrange.packV"),
    m!("Arrange", SEP, ""),
    m!("Arrange", "Round to pixels", "arrange.roundToPixels"),
    // Vector
    m!("Vector", "Join selection", "vector.join"),
    m!("Vector", "Smooth join selection", "vector.smoothJoin"),
    m!("Vector", "Delete and heal selection", "vector.deleteHeal"),
    m!("Vector", "Simplify vector", "vector.simplify"),
    m!("Vector", "Reverse path direction", "vector.reverse"),
    m!("Vector", "Offset path", "vector.offset"),
    // Plugins
    m!("Plugins", "Run last plugin", "plugins.runLast"),
    m!("Plugins", "Manage plugins…", "plugins.manage"),
    m!("Plugins/Development", "Import plugin from manifest…", "plugin.import"),
    // Preferences
    m!("Preferences", "Snap to geometry", "view.snapGeometry"),
    m!("Preferences", "Snap to objects", "view.snapObjects"),
    m!("Preferences", "Snap to pixel grid", "view.snapPixel"),
    m!("Preferences", "Keep tool selected after use", "pref.keepTool"),
    m!("Preferences", "Highlight layers on hover", "pref.highlightHover"),
    m!("Preferences", "Rename duplicated layers", "pref.renameDuplicates"),
    m!("Preferences", "Show dimensions on objects", "pref.showDimensions"),
    m!("Preferences", "Use number keys for opacity", "pref.numberKeysOpacity"),
    m!("Preferences", "Nudge amount…", "pref.nudge"),
    m!("Preferences", "Preferences…", "app.preferences"),
    // Libraries
    m!("Libraries", "Open libraries", "libraries.open"),
    m!("Libraries", "Publish library…", "libraries.publish"),
    m!("Libraries", "Hide when publishing", "component.hideWhenPublishing"),
    // Help
    m!("Help", "Getting started", "help.gettingStarted"),
    m!("Help", "Keyboard shortcuts", "help.shortcuts"),
    m!("Help", "Help page", "help.page"),
    m!("Help", "About FigCraft", "help.about"),
    // Toolbar
    m!("Tools", "Move", "tool.move"),
    m!("Tools", "Scale", "tool.scale"),
    m!("Tools", "Hand", "tool.hand"),
    m!("Tools", "Frame", "tool.frame"),
    m!("Tools", "Section", "tool.section"),
    m!("Tools", "Slice", "tool.slice"),
    m!("Tools", "Rectangle", "tool.rectangle"),
    m!("Tools", "Line", "tool.line"),
    m!("Tools", "Arrow", "tool.arrow"),
    m!("Tools", "Ellipse", "tool.ellipse"),
    m!("Tools", "Polygon", "tool.polygon"),
    m!("Tools", "Star", "tool.star"),
    m!("Tools", "Pen", "tool.pen"),
    m!("Tools", "Pencil", "tool.pencil"),
    m!("Tools", "Text", "tool.text"),
    m!("Tools", "Comment", "tool.comment"),
    m!("Tools", "Shape builder", "tool.shapeBuilder"),
    m!("Tools", "Actions", "app.actions"),
];

/// Items that are real commands (not separators).
pub fn items() -> impl Iterator<Item = &'static MenuItem> {
    MENU.iter().filter(|m| m.label != SEP)
}

/// Items of FigCraft's own that the parity count leaves out (as it does the `collab.` ones).
const OWN: [&str; 3] = ["file.newFromTemplate", "help.gettingStarted", "text.missingFonts"];

/// Coverage: (implemented, total, missing ids). `extra` lists ids implemented by the UI layer.
pub fn parity(extra: &[&str]) -> (usize, usize, Vec<&'static str>) {
    let mut done = 0;
    let mut missing = Vec::new();
    let mut total = 0;
    for m in items().filter(|m| !m.id.starts_with("collab.") && !OWN.contains(&m.id)) {
        total += 1;
        if crate::find_command(m.id).is_some() || extra.contains(&m.id) {
            done += 1;
        } else {
            missing.push(m.id);
        }
    }
    (done, total, missing)
}

/// The default shortcut for a command id on this platform (notation of [`crate::keymap`]).
pub fn shortcut(id: &str) -> Option<&'static str> {
    crate::keymap::primary(id, crate::keymap::Platform::current())
}
