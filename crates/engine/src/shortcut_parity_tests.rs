//! Keyboard shortcut parity with the public shortcut documentation, per platform. One row per
//! documented shortcut: the action in our own words, the macOS and the Windows/Linux keys as the
//! documentation gives them, what does it here (a keymap command, a held key or gesture the panel
//! lists, a vector edit tool key, or nothing yet) and a note. Rows the documentation is silent on
//! pin today's keys, so every default binding is in this table: changing one means changing a row.
//!
//! The documentation is spread over the help centre's feature articles (keyboard layouts, text
//! properties, alignment, frames, components, prototypes, plugins, accessibility…); where two
//! articles disagree the note says so and both keys are bound when they don't collide.

use serde_json::json;

use crate::Session;
use crate::keymap::Group::{self, *};
use crate::keymap::{Context, GESTURES, KEYS, Platform, display, lookup, parse};
use crate::params::id_of;
use crate::vectoredit::VTool;

const PLATFORMS: [Platform; 2] = [Platform::Mac, Platform::Other];

/// What answers a documented shortcut here.
#[derive(Clone, Copy, Debug)]
enum Ours {
    /// The keymap binds the documented keys to this command.
    Bound(&'static str),
    /// Bound to this command on other keys where the documented ones collide with another
    /// documented shortcut: (command, macOS keys, Windows/Linux keys; empty: the macOS keys).
    Moved(&'static str, &'static [&'static str], &'static [&'static str]),
    /// A held key, pointer gesture or presentation key; the shortcuts panel lists it under this label.
    Gesture(&'static str),
    /// A tool key in vector edit mode.
    VectorTool(VTool),
    /// No such behaviour yet (no row today; kept for the next documented shortcut).
    #[allow(dead_code)]
    Missing,
}
use Ours::*;

struct Row {
    /// The shortcuts panel tab the row belongs to.
    group: Group,
    action: &'static str,
    mac: &'static [&'static str],
    /// Empty: the macOS keys with Cmd read as Ctrl.
    win: &'static [&'static str],
    ours: Ours,
    documented: bool,
    /// Who takes the keys before the app on some platform (empty: nobody).
    reserved: &'static str,
    note: &'static str,
}

impl Row {
    const fn reserved(mut self, by: &'static str) -> Self {
        self.reserved = by;
        self
    }
    fn keys(&self, p: Platform) -> &'static [&'static str] {
        if p == Platform::Other && !self.win.is_empty() { self.win } else { self.mac }
    }
    /// The keys that fire the row's command here, and the command.
    fn ours(&self, p: Platform) -> Option<(&'static str, &'static [&'static str])> {
        match self.ours {
            Bound(id) => Some((id, self.keys(p))),
            Moved(id, mac, win) => Some((id, if p == Platform::Other && !win.is_empty() { win } else { mac })),
            _ => None,
        }
    }
}

const fn doc(group: Group, action: &'static str, mac: &'static [&'static str], win: &'static [&'static str], ours: Ours, note: &'static str) -> Row {
    Row { group, action, mac, win, ours, documented: true, reserved: "", note }
}

/// A binding the documentation is silent on: today's keys, pinned.
const fn pin(group: Group, id: &'static str, mac: &'static [&'static str], win: &'static [&'static str]) -> Row {
    Row { group, action: id, mac, win, ours: Bound(id), documented: false, reserved: "", note: "documentation silent: pinned" }
}

const BROWSER_TAB: &str = "browsers keep it for their tabs and windows (web build)";

#[rustfmt::skip]
static ROWS: &[Row] = &[
    // Essential
    doc(Essential, "Open the actions menu", &["Cmd+K", "Cmd+/"], &[], Bound("app.actions"), "Cmd+/ is in the keyboard layout tables"),
    doc(Essential, "Open the keyboard shortcuts panel", &["Ctrl+Shift+?"], &[], Bound("help.shortcuts"), "Control on macOS too (Shift+Cmd+? is the system's Help search)"),
    doc(Essential, "Show or hide the interface", &["Cmd+\\"], &[], Bound("view.toggleUi"), ""),
    doc(Essential, "Find layers and text", &["Cmd+F"], &[], Bound("edit.findReplace"), ""),
    doc(Essential, "Chat at your cursor in a live session", &["/"], &[], Bound("collab.cursorChat"), ""),
    doc(Essential, "Sample a colour", &["I", "Ctrl+C"], &["I"], Bound("edit.pickColor"), "Control+C on macOS only (Ctrl+C copies elsewhere)"),
    doc(Essential, "Save to version history", &["Alt+Cmd+S"], &["Ctrl+Alt+S"], Bound("file.saveVersion"), ""),
    doc(Essential, "Run the last plugin", &["Alt+Cmd+P"], &["Ctrl+Alt+P"], Moved("plugins.runLast", &["Alt+Cmd+P"], &["Ctrl+Alt+Shift+P"]),
        "conflict: the documentation also gives Ctrl+Alt+P to pixel preview on Windows; pixel preview keeps it, the plugin takes Ctrl+Alt+Shift+P"),
    pin(Essential, "view.fullscreen", &["Ctrl+Cmd+F"], &["F11"]),
    pin(Essential, "app.preferences", &["Cmd+,"], &[]),
    pin(Essential, "file.new", &["Cmd+N"], &[]).reserved(BROWSER_TAB),
    pin(Essential, "file.open", &["Cmd+O"], &[]),
    pin(Essential, "file.save", &["Cmd+S"], &[]),
    pin(Essential, "file.saveAs", &["Shift+Cmd+S"], &[]),
    pin(Essential, "file.close", &["Cmd+W"], &[]).reserved(BROWSER_TAB),
    // Tools
    doc(Tools, "Move tool", &["V"], &[], Bound("tool.move"), ""),
    doc(Tools, "Scale tool", &["K"], &[], Bound("tool.scale"), ""),
    doc(Tools, "Frame tool", &["F", "A"], &[], Bound("tool.frame"), "A was unbound before"),
    doc(Tools, "Rectangle tool", &["R"], &[], Bound("tool.rectangle"), ""),
    doc(Tools, "Line tool", &["L"], &[], Bound("tool.line"), ""),
    doc(Tools, "Arrow tool", &["Shift+L"], &[], Bound("tool.arrow"), ""),
    doc(Tools, "Ellipse tool", &["O"], &[], Bound("tool.ellipse"), ""),
    doc(Tools, "Pen tool", &["P"], &[], Bound("tool.pen"), ""),
    doc(Tools, "Pencil tool", &["Shift+P"], &[], Bound("tool.pencil"), ""),
    doc(Tools, "Text tool", &["T"], &[], Bound("tool.text"), ""),
    doc(Tools, "Comment mode", &["C"], &[], Bound("tool.comment"), ""),
    doc(Tools, "Hand tool while held", &["Space"], &[], Gesture("Pan the canvas"), "held, not pressed"),
    doc(Tools, "Pan with the arrows when nothing is selected (Shift: farther)", &["ArrowRight"], &[], Gesture("Pan with nothing selected (Shift: farther)"),
        "view.pan; nudges move the selection when there is one"),
    doc(Tools, "Place an object with the tool chosen from the toolbar", &["Enter"], &[], Gesture("Place a shape or frame (its tool chosen)"),
        "tool.place: Enter on the toolbar's chosen tool or on the canvas; a click's size (100×100, lines 100 long, sections 400×300), selected; no crosshair to move first"),
    pin(Tools, "tool.hand", &["H"], &[]),
    pin(Tools, "tool.section", &["Shift+S"], &[]),
    pin(Tools, "tool.slice", &["S"], &[]),
    pin(Tools, "file.placeImage", &["Shift+Cmd+K"], &[]),
    pin(Tools, "tool.shapeBuilder", &["Shift+M"], &[]),
    // Board tools (boards only), on keys no design-file shortcut uses.
    pin(Tools, "tool.sticky", &["Q"], &[]),
    pin(Tools, "tool.shape", &["B"], &[]),
    pin(Tools, "tool.connector", &["X"], &[]),
    pin(Tools, "tool.marker", &["M"], &[]),
    pin(Tools, "tool.highlighter", &["U"], &[]),
    pin(Tools, "tool.eraser", &["E"], &[]),
    pin(Tools, "tool.stamp", &["W"], &[]),
    pin(Tools, "board.nextSticky", &["Shift+Cmd+Enter"], &[]),
    // Vector edit mode
    doc(Selection, "Edit the selected shape's points", &["Enter"], &[], Bound("select.children"), "Enter on a shape edits it; on a frame or group it selects the children"),
    doc(Shape, "Vector edit: move", &["V"], &[], VectorTool(VTool::Move), ""),
    doc(Shape, "Vector edit: pen", &["P"], &[], VectorTool(VTool::Pen), ""),
    doc(Shape, "Vector edit: cut", &["X"], &[], VectorTool(VTool::Cut), ""),
    doc(Shape, "Vector edit: eraser", &["Shift+E"], &[], VectorTool(VTool::Eraser), ""),
    doc(Shape, "Vector edit: lasso", &["Q"], &[], VectorTool(VTool::Lasso), ""),
    doc(Shape, "Vector edit: paint bucket", &["Shift+B"], &[], VectorTool(VTool::Paint), "B alone works too (pinned)"),
    // View
    doc(View, "Pixel grid", &["Shift+'", "Cmd+'"], &[], Bound("view.pixelGrid"),
        "articles disagree (Shift+' in most, Cmd+' in the zoom article); both bound, Shift+' first"),
    doc(View, "Snap to the pixel grid", &["Shift+Cmd+'"], &[], Bound("view.snapPixel"), ""),
    doc(View, "Layout guides", &["Ctrl+G", "Shift+G"], &["Ctrl+Shift+4", "Shift+G"], Bound("view.layoutGrids"), ""),
    doc(View, "Outline mode", &["Shift+Cmd+O"], &[], Bound("view.outlines"), ""),
    doc(View, "Pixel preview", &["Ctrl+P"], &["Ctrl+Alt+P"], Bound("view.pixelPreview"), ""),
    doc(View, "Others' cursors in a live session", &["Alt+Cmd+\\"], &["Ctrl+Alt+\\"], Bound("view.multiplayerCursors"), "new command"),
    doc(View, "Hide the side panels (minimize the interface)", &["Shift+Cmd+\\"], &[], Bound("view.leftSidebar"), ""),
    doc(View, "Layers panel", &["Alt+1"], &[], Bound("view.layers"), "one article writes Ctrl+1 for Windows, another Alt+1; Alt+1 bound"),
    doc(View, "Assets panel", &["Alt+2"], &[], Bound("view.assets"), ""),
    doc(View, "Libraries", &["Alt+3"], &[], Bound("libraries.open"), ""),
    doc(View, "Dev Mode on or off", &["Shift+D"], &[], Bound("view.devMode"), ""),
    doc(View, "Move the keyboard to the toolbar (then the next region)", &["F6"], &["Ctrl+F6"], Bound("view.nextRegion"),
        "F6 works on Windows and Linux too; F6 and F8 need Fn on many Mac keyboards").reserved("browsers move to the address bar on F6 (web build)"),
    doc(View, "Move along the toolbar and the layer rows", &["ArrowRight"], &[], Gesture("Move along the toolbar or the layer rows"), ""),
    pin(View, "view.rulers", &["Shift+R"], &[]),
    pin(View, "view.comments", &["Shift+C"], &[]),
    pin(View, "view.design", &["Alt+8"], &[]),
    pin(View, "view.prototype", &["Alt+9"], &[]),
    pin(View, "view.designPrototype", &["Shift+E"], &[]),
    pin(View, "view.nextRegion", &["F6"], &["F6"]),
    pin(View, "view.previousRegion", &["Shift+F6"], &[]),
    pin(View, "view.contextMenu", &["Shift+F10"], &[]),
    pin(View, "layers.collapseAll", &["Alt+L"], &[]),
    pin(View, "doc.nextTab", &["Ctrl+Tab"], &[]).reserved(BROWSER_TAB),
    pin(View, "doc.previousTab", &["Ctrl+Shift+Tab"], &[]).reserved(BROWSER_TAB),
    pin(View, "view.prevPage", &["PageUp"], &[]),
    pin(View, "view.nextPage", &["PageDown"], &[]),
    // Zoom
    doc(Zoom, "Zoom in", &["Cmd+=", "Shift+="], &[], Bound("view.zoomIn"), "`+` with or without Cmd/Ctrl"),
    doc(Zoom, "Zoom out", &["Cmd+-", "-"], &[], Bound("view.zoomOut"), ""),
    doc(Zoom, "Zoom to fit", &["Shift+1"], &[], Bound("view.zoomFit"), ""),
    doc(Zoom, "Zoom to the selection", &["Shift+2"], &[], Bound("view.zoomSelection"), ""),
    pin(Zoom, "view.zoomIn", &["Shift+Cmd+=", "="], &[]),
    pin(Zoom, "view.zoomOut", &["Shift+-", "Shift+Cmd+-"], &[]),
    pin(Zoom, "view.zoom100", &["Shift+0", "Cmd+0"], &[]),
    pin(Zoom, "view.prevFrame", &["Shift+N", "Home"], &[]),
    pin(Zoom, "view.nextFrame", &["N", "End"], &[]),
    // Text
    doc(Text, "Bold", &["Cmd+B"], &[], Bound("text.bold"), ""),
    doc(Text, "Italic", &["Cmd+I"], &[], Bound("text.italic"), ""),
    doc(Text, "Underline", &["Cmd+U"], &[], Bound("text.underline"), ""),
    doc(Text, "Strikethrough", &["Shift+Cmd+X"], &[], Bound("text.strikethrough"), ""),
    doc(Text, "Link the selected text", &["Shift+Cmd+U"], &[], Bound("text.link"), "while editing; Cmd+K (the comment composer's link key) kept too"),
    doc(Text, "Numbered list", &["Shift+Cmd+7"], &[], Bound("text.numberList"), ""),
    doc(Text, "Bulleted list", &["Shift+Cmd+8"], &[], Bound("text.bulletList"), ""),
    doc(Text, "List items a level deeper", &["Cmd+]"], &[], Bound("text.listIndent"), "while editing (Cmd+] brings forward otherwise); Tab too"),
    doc(Text, "List items a level out", &["Cmd+["], &[], Bound("text.listOutdent"), "while editing; Shift+Tab too"),
    doc(Text, "Larger text", &["Shift+Cmd+."], &[], Bound("text.sizeUp"), "written Shift+Cmd+>"),
    doc(Text, "Smaller text", &["Shift+Cmd+,"], &[], Bound("text.sizeDown"), "written Shift+Cmd+<"),
    doc(Text, "Heavier weight", &["Alt+Cmd+."], &[], Bound("text.weightUp"), ""),
    doc(Text, "Lighter weight", &["Alt+Cmd+,"], &[], Bound("text.weightDown"), ""),
    doc(Text, "More letter spacing", &["Alt+."], &[], Bound("text.trackingUp"), "the text article writes Alt; one layout table writes Cmd, which would be the font size keys"),
    doc(Text, "Less letter spacing", &["Alt+,"], &[], Bound("text.trackingDown"), ""),
    doc(Text, "More line height", &["Alt+Shift+."], &[], Bound("text.lineHeightUp"), ""),
    doc(Text, "Less line height", &["Alt+Shift+,"], &[], Bound("text.lineHeightDown"), ""),
    pin(Text, "text.link", &["Cmd+K"], &[]),
    pin(Text, "text.alignLeft", &["Alt+Cmd+L"], &[]).reserved("some Linux desktops lock the screen on Ctrl+Alt+L"),
    pin(Text, "text.alignCenter", &["Alt+Cmd+T"], &[]).reserved("Ubuntu opens a terminal on Ctrl+Alt+T"),
    pin(Text, "text.alignRight", &["Alt+Cmd+R"], &[]),
    pin(Text, "text.alignJustify", &["Alt+Cmd+J"], &[]),
    // Shape
    doc(Shape, "Union", &["Alt+Shift+U"], &[], Bound("bool.union"), ""),
    doc(Shape, "Subtract", &["Alt+Shift+S"], &[], Bound("bool.subtract"), ""),
    doc(Shape, "Intersect", &["Alt+Shift+I"], &[], Bound("bool.intersect"), ""),
    doc(Shape, "Exclude", &["Alt+Shift+E"], &[], Bound("bool.exclude"), ""),
    doc(Shape, "Flatten", &["Alt+Shift+F"], &[], Bound("vector.flatten"), ""),
    doc(Shape, "Remove fill", &["Alt+/"], &[], Bound("object.removeFill"), "one layout table writes Alt+, for Windows; Alt+, is letter spacing"),
    doc(Shape, "Remove stroke", &["Shift+/"], &[], Bound("object.removeStroke"), ""),
    pin(Shape, "vector.flatten", &["Cmd+E"], &[]),
    pin(Shape, "vector.outlineStroke", &["Alt+Cmd+O"], &[]),
    pin(Shape, "object.swapFillStroke", &["Shift+X"], &[]),
    pin(Shape, "vector.join", &["Cmd+J"], &[]),
    pin(Shape, "vector.smoothJoin", &["Shift+Cmd+J"], &[]),
    pin(Shape, "vector.deleteHeal", &["Shift+Delete"], &[]),
    // Selection
    doc(Selection, "Select all", &["Cmd+A"], &[], Bound("select.all"), ""),
    doc(Selection, "Invert the selection", &["Shift+Cmd+A"], &[], Bound("select.inverse"), ""),
    doc(Selection, "Select matching layers", &["Alt+Cmd+A"], &["Ctrl+Alt+A"], Bound("select.matching"), ""),
    doc(Selection, "Deselect", &["Escape"], &[], Bound("select.none"), "steps out of editing first"),
    doc(Selection, "Select the parent", &["Shift+Enter", "\\"], &[], Bound("select.parent"), ""),
    doc(Selection, "Next sibling", &["Tab"], &[], Bound("select.next"), ""),
    doc(Selection, "Previous sibling", &["Shift+Tab"], &[], Bound("select.previous"), ""),
    doc(Selection, "Group", &["Cmd+G"], &[], Bound("object.group"), ""),
    doc(Selection, "Ungroup", &["Shift+Cmd+G", "Cmd+Delete"], &[], Bound("object.ungroup"), "Cmd/Ctrl+Backspace was unbound before"),
    doc(Selection, "Frame the selection", &["Alt+Cmd+G"], &["Ctrl+Alt+G"], Bound("object.frameSelection"), ""),
    doc(Selection, "Hide or show", &["Shift+Cmd+H"], &[], Bound("object.toggleVisible"), ""),
    doc(Selection, "Lock or unlock", &["Shift+Cmd+L"], &[], Bound("object.toggleLock"), ""),
    doc(Selection, "Rename (several layers: batch rename)", &["Cmd+R"], &[], Bound("object.rename"), ""),
    doc(Selection, "Keyboard box selection", &["Alt+Space"], &["Ctrl+Space"], Bound("select.area"),
        "then the arrows move it, Cmd/Ctrl+arrows draw it out (Shift: farther), Enter selects, Escape closes; F8 kept")
        .reserved("Ctrl+Space switches the input method with some Chinese, Japanese and Korean input methods on Windows and Linux"),
    doc(Selection, "Select nested layers", &["Cmd"], &[], Gesture("Deep select"), "held while clicking or dragging"),
    pin(Selection, "object.rename", &["F2"], &[]),
    pin(Selection, "select.area", &["F8"], &[]),
    // Edit
    doc(Edit, "Undo", &["Cmd+Z"], &[], Bound("edit.undo"), ""),
    doc(Edit, "Copy", &["Cmd+C"], &[], Bound("edit.copy"), ""),
    doc(Edit, "Paste", &["Cmd+V"], &[], Bound("edit.paste"), ""),
    doc(Edit, "Duplicate", &["Cmd+D"], &[], Bound("edit.duplicate"), ""),
    doc(Edit, "Delete", &["Delete"], &[], Bound("edit.delete"), "Backspace or Delete"),
    doc(Edit, "Paste over the selection", &["Shift+Cmd+V"], &[], Bound("edit.pasteOver"), ""),
    doc(Edit, "Paste to replace", &["Shift+Cmd+R"], &[], Bound("edit.pasteReplace"), ""),
    doc(Edit, "Copy as PNG", &["Shift+Cmd+C"], &[], Bound("edit.copyAsPng"), ""),
    doc(Edit, "Copy all properties", &["Alt+Cmd+C"], &["Ctrl+Alt+C"], Bound("edit.copyProperties"), ""),
    doc(Edit, "Paste all properties", &["Alt+Cmd+V"], &["Ctrl+Alt+V"], Bound("edit.pasteProperties"), ""),
    doc(Edit, "Export", &["Shift+Cmd+E"], &[], Bound("file.export"), ""),
    doc(Edit, "Duplicate by dragging", &["Alt"], &[], Gesture("Duplicate by dragging"), "held while dragging"),
    pin(Edit, "edit.redo", &["Shift+Cmd+Z"], &[]),
    pin(Edit, "edit.cut", &["Cmd+X"], &[]),
    pin(Edit, "file.import", &["Shift+Cmd+I"], &[]),
    // Transform
    doc(Transform, "Flip horizontally", &["Shift+H"], &[], Bound("object.flipH"), ""),
    doc(Transform, "Flip vertically", &["Shift+V"], &[], Bound("object.flipV"), ""),
    doc(Transform, "Use as mask (again: release)", &["Ctrl+Cmd+M"], &["Ctrl+Alt+M"], Bound("object.mask"), ""),
    doc(Transform, "Resize a frame to fit its contents", &["Alt+Shift+Cmd+R"], &["Ctrl+Alt+Shift+R"], Bound("object.resizeToFit"), ""),
    doc(Transform, "Nudge (Shift: big nudge)", &["ArrowRight"], &[], Gesture("Nudge (Shift: big nudge)"), ""),
    doc(Transform, "Suspend snapping while dragging", &["Ctrl"], &[], Gesture("Suspend snapping while dragging"),
        "Control on macOS (Cmd keeps working); Ctrl elsewhere"),
    doc(Transform, "Show the rotation origin to move it", &["Alt+R"], &[], Bound("transform.rotationOrigin"),
        "drag it (snaps to the box's corners, edge middles and centre), double-click: back to the centre, Escape hides it; kept while the selection lasts, not saved (the documentation doesn't say)"),
    // Arrange
    doc(Arrange, "Bring forward", &["Cmd+]"], &[], Bound("arrange.forward"), ""),
    doc(Arrange, "Send backward", &["Cmd+["], &[], Bound("arrange.backward"), ""),
    doc(Arrange, "Bring to front", &["Alt+Cmd+]"], &["Ctrl+Shift+]"], Bound("arrange.front"), "one layout table shows only the bracket"),
    doc(Arrange, "Send to back", &["Alt+Cmd+["], &["Ctrl+Shift+["], Bound("arrange.back"), ""),
    doc(Arrange, "Align left", &["Alt+A"], &[], Bound("align.left"), ""),
    doc(Arrange, "Align horizontal centres", &["Alt+H"], &[], Bound("align.hcenter"), ""),
    doc(Arrange, "Align right", &["Alt+D"], &[], Bound("align.right"), ""),
    doc(Arrange, "Align top", &["Alt+W"], &[], Bound("align.top"), ""),
    doc(Arrange, "Align vertical centres", &["Alt+V"], &[], Bound("align.vcenter"), ""),
    doc(Arrange, "Align bottom", &["Alt+S"], &[], Bound("align.bottom"), ""),
    doc(Arrange, "Add auto layout", &["Shift+A"], &[], Bound("layout.add"), ""),
    doc(Arrange, "Remove auto layout", &["Alt+Shift+A"], &[], Bound("layout.remove"), ""),
    doc(Arrange, "Suggest auto layout", &["Ctrl+Shift+A"], &["Ctrl+Alt+Shift+A"], Bound("layout.suggest"), "was unbound"),
    pin(Arrange, "distribute.horizontal", &["Ctrl+Alt+H"], &["Alt+Shift+H"]),
    pin(Arrange, "distribute.vertical", &["Ctrl+Alt+V"], &["Alt+Shift+V"]),
    pin(Arrange, "arrange.tidy", &["Ctrl+Alt+T"], &["Ctrl+Alt+Shift+T"]),
    // Components
    doc(Components, "Create a component", &["Alt+Cmd+K"], &["Ctrl+Alt+K"], Bound("component.create"), ""),
    doc(Components, "Detach an instance", &["Alt+Cmd+B"], &["Ctrl+Alt+B"], Bound("component.detach"), ""),
    doc(Components, "Go to the main component", &["Ctrl+Alt+Cmd+K"], &["Ctrl+Alt+Shift+K"], Bound("component.goToMain"), "was unbound"),
    doc(Components, "Quick insert", &["Shift+I"], &[], Bound("view.assets"), "opens the Assets panel's search"),
    // Prototype
    doc(Prototype, "Preview in the canvas", &["Shift+Space"], &[], Bound("proto.preview"), ""),
    doc(Prototype, "Present", &["Alt+Cmd+Enter"], &["Ctrl+Alt+Enter"], Bound("proto.present"), ""),
    doc(Prototype, "Presenting: restart", &["R"], &[], Gesture("Presenting: restart"), "the inline preview too"),
    doc(Prototype, "Presenting: next frame", &["ArrowRight", "Space", "N"], &[], Gesture("Presenting: next frame"), "Space and N were unbound"),
    doc(Prototype, "Presenting: previous frame", &["ArrowLeft"], &[], Gesture("Presenting: previous frame"), ""),
    doc(Prototype, "Presenting: cycle the size options", &["Z"], &[], Gesture("Presenting: cycle the size options"), ""),
    doc(Prototype, "Presenting: comments", &["C"], &[], Gesture("Presenting: comments"), "Shift+C kept; keys the prototype uses go to it first"),
    doc(Prototype, "Presenting: full screen", &["F"], &[], Gesture("Presenting: full screen"), "new"),
];

/// Chords the operating system takes before any app sees them; no default binding may use one.
const OS_RESERVED: &[(Platform, &str, &str)] = &[
    (Platform::Mac, "Cmd+H", "hide the app"),
    (Platform::Mac, "Cmd+M", "minimize"),
    (Platform::Mac, "Cmd+Q", "quit"),
    (Platform::Mac, "Cmd+Tab", "app switcher"),
    (Platform::Mac, "Cmd+Space", "Spotlight"),
    (Platform::Mac, "Ctrl+Space", "next input source"),
    (Platform::Mac, "Alt+Cmd+Escape", "force quit"),
    (Platform::Mac, "Shift+Cmd+3", "screenshot"),
    (Platform::Mac, "Shift+Cmd+4", "screenshot"),
    (Platform::Mac, "Shift+Cmd+5", "screenshot"),
    (Platform::Mac, "Shift+Cmd+/", "Help menu search"),
    (Platform::Mac, "Ctrl+ArrowLeft", "Mission Control spaces"),
    (Platform::Mac, "Ctrl+ArrowRight", "Mission Control spaces"),
    (Platform::Mac, "Ctrl+ArrowUp", "Mission Control"),
    (Platform::Mac, "Ctrl+ArrowDown", "App Exposé"),
    (Platform::Other, "Alt+Space", "the window menu"),
    (Platform::Other, "Alt+Tab", "window switcher"),
    (Platform::Other, "Alt+F4", "close the window"),
    (Platform::Other, "Ctrl+Escape", "Start menu"),
    (Platform::Other, "Ctrl+Shift+Escape", "Task Manager"),
    (Platform::Other, "Ctrl+Alt+Delete", "security screen"),
    (Platform::Other, "Ctrl+Alt+ArrowLeft", "Linux workspaces"),
    (Platform::Other, "Ctrl+Alt+ArrowRight", "Linux workspaces"),
    (Platform::Other, "Ctrl+Alt+F1", "Linux virtual terminals"),
    (Platform::Other, "Ctrl+Alt+F2", "Linux virtual terminals"),
];

/// A chord as the platform sees it (Cmd and Ctrl are one key off macOS).
fn chord(p: Platform, spec: &str) -> (bool, bool, bool, bool, String) {
    let (m, k) = parse(spec).unwrap_or_else(|| panic!("`{spec}` doesn't parse"));
    let (cmd, ctrl) = match p {
        Platform::Mac => (m.cmd, m.ctrl),
        Platform::Other => (m.cmd || m.ctrl, false),
    };
    (m.shift, m.alt, cmd, ctrl, k.to_ascii_uppercase())
}

/// The commands a chord fires on the canvas and while editing text.
fn fires(p: Platform, spec: &str) -> [Option<&'static str>; 2] {
    let (m, k) = parse(spec).unwrap_or_else(|| panic!("`{spec}` doesn't parse"));
    [Context::Canvas, Context::Text].map(|c| lookup(p, c, k, m).map(|b| b.id))
}

/// Keys of gesture rows name a key, not a chord to look up.
fn is_chord(spec: &str) -> bool {
    !matches!(spec, "Cmd" | "Alt" | "Ctrl")
}

#[test]
fn documented_shortcuts_fire_their_commands_on_both_platforms() {
    for row in ROWS {
        for p in PLATFORMS {
            assert!(row.keys(p).iter().all(|k| !is_chord(k) || parse(k).is_some()), "{}: keys don't parse", row.action);
            if let Some((id, keys)) = row.ours(p) {
                assert!(!keys.is_empty(), "{}", row.action);
                for k in keys {
                    assert!(fires(p, k).contains(&Some(id)), "{p:?}: `{k}` should run {id} ({}), runs {:?}", row.action, fires(p, k));
                }
                // The panel lists the binding in the row's tab.
                assert!(KEYS.iter().any(|b| b.id == id && b.group == row.group), "{id} isn't in the {:?} tab", row.group);
            }
            if let Moved(id, ..) = row.ours {
                let ours = row.ours(p).map(|o| o.1).unwrap_or_default();
                for k in row.keys(p).iter().filter(|k| !ours.contains(k)) {
                    assert!(!fires(p, k).contains(&Some(id)), "{p:?}: `{k}` was moved off {id}");
                    assert!(fires(p, k).iter().any(Option::is_some), "{p:?}: `{k}` should be the conflicting documented command");
                }
            }
        }
        if let Gesture(label) = row.ours {
            assert!(GESTURES.iter().any(|g| g.0 == row.group && g.1 == label), "the panel's {:?} tab doesn't list `{label}`", row.group);
        }
        assert!(row.documented || !row.note.is_empty());
    }
}

#[test]
fn vector_edit_tool_keys() {
    for row in ROWS {
        let VectorTool(tool) = row.ours else { continue };
        let mut s = Session::new();
        let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        s.execute("vector.edit.begin", &json!({"id": id_of(&r["id"]).unwrap()})).unwrap();
        for k in row.mac {
            let (m, key) = parse(k).unwrap();
            assert!(crate::vectoredit::key(&mut s, key, m.shift, m.alt, m.cmd).unwrap(), "`{k}` taken");
            assert_eq!(s.ds().unwrap().vector_edit.as_ref().unwrap().tool, tool, "`{k}`");
        }
        assert_eq!(tool.shortcut(), row.mac[0], "the tooltip shows the documented key");
    }
}

#[test]
fn every_default_binding_is_in_the_table() {
    for b in KEYS {
        for p in PLATFORMS {
            for k in b.keys(p) {
                let listed = ROWS.iter().filter_map(|r| r.ours(p)).any(|(id, keys)| id == b.id && keys.iter().any(|x| chord(p, x) == chord(p, k)));
                assert!(listed, "{p:?}: `{k}` ({}) has no row: add it, documented or pinned", b.id);
            }
        }
    }
}

#[test]
fn counts_match_the_docs() {
    let documented: Vec<&Row> = ROWS.iter().filter(|r| r.documented).collect();
    let bound = documented.iter().filter(|r| matches!(r.ours, Bound(_) | Gesture(_) | VectorTool(_))).count();
    let moved = documented.iter().filter(|r| matches!(r.ours, Moved(..))).count();
    let missing = documented.iter().filter(|r| matches!(r.ours, Missing)).count();
    let reserved = documented.iter().filter(|r| !r.reserved.is_empty()).count();
    let pinned = ROWS.len() - documented.len();
    // docs/ui-parity.md quotes these.
    assert_eq!((documented.len(), bound, moved, missing, reserved, pinned), (129, 128, 1, 0, 2, 57));
}

#[test]
fn no_binding_takes_a_chord_the_system_keeps() {
    for (p, spec, what) in OS_RESERVED {
        for b in KEYS {
            assert!(b.keys(*p).iter().all(|k| chord(*p, k) != chord(*p, spec)), "{p:?}: {} uses `{spec}` ({what})", b.id);
        }
    }
}

#[test]
fn modifiers_read_the_platform_way() {
    for b in KEYS {
        for k in b.keys(Platform::Mac) {
            let shown = display(k, Platform::Mac);
            assert!(!shown.contains("Ctrl") && !shown.contains("Alt"), "{}: macOS shows `{shown}`", b.id);
        }
        for k in b.keys(Platform::Other) {
            let shown = display(k, Platform::Other);
            assert!(!shown.contains(['⌘', '⌥', '⌃', '⇧']), "{}: Windows/Linux shows `{shown}`", b.id);
            // Windows and Linux have no Command key: Cmd and Ctrl both mean Ctrl there.
            assert!(!(k.contains("Cmd") && k.contains("Ctrl")), "{}: `{k}` names Ctrl twice off macOS", b.id);
        }
        assert!(b.keys(Platform::Mac).iter().chain(b.keys(Platform::Other)).all(|k| !k.contains("Option") && !k.contains("Win")));
    }
}

#[test]
fn control_held_while_dragging_suspends_snapping() {
    use crate::tools::{PointerEvent, PointerKind};
    let mut s = Session::new();
    for x in [0, 400] {
        s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap();
    }
    let b = id_of(&s.execute("shape.rectangle", &json!({"x": 200, "y": 300, "width": 100, "height": 100})).unwrap()["id"]).unwrap();
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    let ev = |kind, x, y, ctrl| {
        let mut e = PointerEvent::at(kind, x, y, 1.0);
        e.ctrl = ctrl;
        e
    };
    // Grabbed at its centre and dropped 2 px off equal gaps between the other two.
    for (ctrl, want) in [(false, (200.0, 0.0)), (true, (198.0, 2.0))] {
        s.pointer(&ev(PointerKind::Down, 250.0, 350.0, ctrl)).unwrap();
        s.pointer(&ev(PointerKind::Move, 248.0, 52.0, ctrl)).unwrap();
        s.pointer(&ev(PointerKind::Up, 248.0, 52.0, ctrl)).unwrap();
        let r = s.doc().unwrap().world_bounds(b).unwrap();
        assert_eq!((r.x0, r.y0), want, "Control held: {ctrl}");
        s.execute("edit.undo", &json!({})).unwrap();
    }
}
