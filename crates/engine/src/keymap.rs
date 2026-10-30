//! Default keyboard shortcuts, grouped like the public shortcut reference, with the macOS keys and
//! the Windows/Linux keys where they differ. The menus, the Keyboard shortcuts panel, the canvas,
//! the control channel and the MCP `key` tool all resolve keys here, so a binding is defined once.
//!
//! Notation: modifiers then the key, joined by `+` (`Shift+Cmd+K`). `Cmd` is the platform's
//! command key (⌘ on macOS, Ctrl elsewhere), `Ctrl` the Control key (on Windows and Linux the same
//! key as `Cmd`), `Alt` is ⌥ Option on macOS. Keys: letters, digits, `= - [ ] \ ' , . / ;` and
//! the names `Enter`, `Escape`, `Tab`, `Delete` (Backspace or Delete), `Space`, `PageUp`,
//! `PageDown`, `Home`, `End`, `ArrowLeft`/`Right`/`Up`/`Down`, `F1`…`F12`. Shifted symbols
//! (`?`, `+`, `<`, `>`, `{`, `}`, `|`, `!`) stand for the unshifted key with Shift held.

/// Which platform's keys to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub enum Platform {
    Mac,
    /// Windows and Linux.
    Other,
}

impl Platform {
    /// The platform this build runs on (the web build asks the browser instead, in the UI).
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") { Self::Mac } else { Self::Other }
    }
}

/// The tabs of the Keyboard shortcuts panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Essential,
    Tools,
    View,
    Zoom,
    Text,
    Shape,
    Selection,
    Edit,
    Transform,
    Arrange,
    Components,
    Prototype,
}

impl Group {
    pub const ALL: [Group; 12] = [
        Group::Essential,
        Group::Tools,
        Group::View,
        Group::Zoom,
        Group::Text,
        Group::Shape,
        Group::Selection,
        Group::Edit,
        Group::Transform,
        Group::Arrange,
        Group::Components,
        Group::Prototype,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Group::Essential => "Essential",
            Group::Tools => "Tools",
            Group::View => "View",
            Group::Zoom => "Zoom",
            Group::Text => "Text",
            Group::Shape => "Shape",
            Group::Selection => "Selection",
            Group::Edit => "Edit",
            Group::Transform => "Transform",
            Group::Arrange => "Arrange",
            Group::Components => "Components",
            Group::Prototype => "Prototype",
        }
    }
}

/// Where a binding is live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// On the canvas (not while editing text).
    Canvas,
    /// Only while editing text.
    Text,
    /// On the canvas and while editing text.
    Both,
}

/// One command's default keys.
#[derive(Clone, Copy, Debug)]
pub struct Binding {
    pub group: Group,
    /// The command id (an engine command or one the UI handles).
    pub id: &'static str,
    /// The label in the shortcuts panel; empty: the command's menu label.
    label: &'static str,
    /// macOS keys, the first shown in menus.
    pub mac: &'static [&'static str],
    /// Windows/Linux keys; empty: the macOS keys with `Cmd` read as Ctrl.
    pub other: &'static [&'static str],
    pub context: Context,
}

impl Binding {
    const fn new(group: Group, id: &'static str, mac: &'static [&'static str]) -> Self {
        Self { group, id, label: "", mac, other: &[], context: Context::Canvas }
    }
    /// Different keys on Windows and Linux.
    const fn win(mut self, keys: &'static [&'static str]) -> Self {
        self.other = keys;
        self
    }
    const fn label(mut self, label: &'static str) -> Self {
        self.label = label;
        self
    }
    /// Live while editing text too.
    const fn both(mut self) -> Self {
        self.context = Context::Both;
        self
    }
    /// Live only while editing text.
    const fn text(mut self) -> Self {
        self.context = Context::Text;
        self
    }

    /// The keys on `platform`, the first being the one menus show.
    pub fn keys(&self, platform: Platform) -> &'static [&'static str] {
        match platform {
            Platform::Other if !self.other.is_empty() => self.other,
            _ => self.mac,
        }
    }

    /// The label for the shortcuts panel: its own, else the menu's, else the command's.
    pub fn title(&self) -> &'static str {
        if !self.label.is_empty() {
            return self.label;
        }
        crate::catalog::items()
            .find(|m| m.id == self.id)
            .map(|m| m.label)
            .or_else(|| crate::find_command(self.id).map(|c| c.label))
            .unwrap_or(self.id)
    }

    fn live_in(&self, context: Context) -> bool {
        self.context == context || self.context == Context::Both
    }
}

use Group::*;

const fn k(group: Group, id: &'static str, mac: &'static [&'static str]) -> Binding {
    Binding::new(group, id, mac)
}

/// Every default shortcut, in panel order within each group.
pub static KEYS: &[Binding] = &[
    // Essential
    k(Essential, "app.actions", &["Cmd+K", "Cmd+/"]).label("Actions…"),
    k(Essential, "help.shortcuts", &["Ctrl+Shift+?"]).both(),
    k(Essential, "view.toggleUi", &["Cmd+\\"]),
    k(Essential, "view.fullscreen", &["Ctrl+Cmd+F"]).win(&["F11"]),
    k(Essential, "edit.pickColor", &["I", "Ctrl+C"]).win(&["I"]),
    k(Essential, "app.preferences", &["Cmd+,"]),
    k(Essential, "file.new", &["Cmd+N"]),
    k(Essential, "file.open", &["Cmd+O"]),
    k(Essential, "file.save", &["Cmd+S"]).both(),
    k(Essential, "file.saveAs", &["Shift+Cmd+S"]).both(),
    k(Essential, "file.saveVersion", &["Alt+Cmd+S"]),
    k(Essential, "file.close", &["Cmd+W"]),
    k(Essential, "edit.findReplace", &["Cmd+F"]),
    // In a live session.
    k(Essential, "collab.cursorChat", &["/"]),
    // Ctrl+Alt+P is Pixel preview on Windows and Linux here.
    k(Essential, "plugins.runLast", &["Alt+Cmd+P"]).win(&["Ctrl+Alt+Shift+P"]),
    // Tools
    k(Tools, "tool.move", &["V"]),
    k(Tools, "tool.scale", &["K"]),
    k(Tools, "tool.hand", &["H"]),
    k(Tools, "tool.frame", &["F", "A"]),
    k(Tools, "tool.section", &["Shift+S"]),
    k(Tools, "tool.slice", &["S"]),
    k(Tools, "tool.rectangle", &["R"]),
    k(Tools, "tool.line", &["L"]),
    k(Tools, "tool.arrow", &["Shift+L"]),
    k(Tools, "tool.ellipse", &["O"]),
    k(Tools, "file.placeImage", &["Shift+Cmd+K"]).label("Place image or video"),
    k(Tools, "tool.pen", &["P"]),
    k(Tools, "tool.pencil", &["Shift+P"]),
    k(Tools, "tool.text", &["T"]),
    k(Tools, "tool.comment", &["C"]),
    k(Tools, "tool.shapeBuilder", &["Shift+M"]),
    // Board tools (on boards only), on keys nothing else uses.
    k(Tools, "tool.sticky", &["Q"]),
    k(Tools, "tool.shape", &["B"]),
    k(Tools, "tool.connector", &["X"]),
    k(Tools, "tool.marker", &["M"]),
    k(Tools, "tool.highlighter", &["U"]),
    k(Tools, "tool.eraser", &["E"]),
    k(Tools, "tool.stamp", &["W"]),
    k(Tools, "board.nextSticky", &["Shift+Cmd+Enter"]).label("Board: a new sticky beside the selected one, or the next shape joined to it").both(),
    // View
    // The documentation gives both keys for the pixel grid in different places.
    k(View, "view.pixelGrid", &["Shift+'", "Cmd+'"]),
    k(View, "view.layoutGrids", &["Shift+G", "Ctrl+G"]).win(&["Shift+G", "Ctrl+Shift+4"]),
    k(View, "view.rulers", &["Shift+R"]),
    k(View, "view.outlines", &["Shift+Cmd+O"]),
    k(View, "view.pixelPreview", &["Ctrl+P"]).win(&["Ctrl+Alt+P"]),
    k(View, "view.comments", &["Shift+C"]),
    k(View, "view.multiplayerCursors", &["Alt+Cmd+\\"]),
    k(View, "view.snapPixel", &["Shift+Cmd+'"]),
    k(View, "view.layers", &["Alt+1"]).label("Layers panel"),
    k(View, "view.assets", &["Alt+2"]).label("Assets panel"),
    k(View, "libraries.open", &["Alt+3"]),
    k(View, "view.design", &["Alt+8"]).label("Design panel"),
    k(View, "view.prototype", &["Alt+9"]).label("Prototype panel"),
    k(View, "view.designPrototype", &["Shift+E"]),
    k(View, "view.devMode", &["Shift+D"]).label("Dev Mode on/off"),
    k(View, "view.leftSidebar", &["Shift+Cmd+\\"]),
    k(View, "view.nextRegion", &["F6"]).win(&["F6", "Ctrl+F6"]).label("Focus the next region (toolbar, layers, canvas, properties)").both(),
    k(View, "view.previousRegion", &["Shift+F6"]).label("Focus the previous region").both(),
    k(View, "view.contextMenu", &["Shift+F10", "Menu"]).label("Context menu of the selection (the focused layer row or the canvas)"),
    k(View, "layers.collapseAll", &["Alt+L"]),
    k(View, "doc.nextTab", &["Ctrl+Tab"]),
    k(View, "doc.previousTab", &["Ctrl+Shift+Tab"]),
    k(View, "view.prevPage", &["PageUp"]),
    k(View, "view.nextPage", &["PageDown"]),
    // Zoom
    // Each zoom-in key has its zoom-out twin on the key beside it: `-` alone zooms out, so `=`
    // alone (the `+` key without Shift) zooms in (#973).
    k(Zoom, "view.zoomIn", &["Cmd+=", "Shift+=", "Shift+Cmd+=", "="]),
    k(Zoom, "view.zoomOut", &["Cmd+-", "-", "Shift+-", "Shift+Cmd+-"]),
    k(Zoom, "view.zoom100", &["Shift+0", "Cmd+0"]),
    k(Zoom, "view.zoomFit", &["Shift+1"]),
    k(Zoom, "view.zoomSelection", &["Shift+2"]),
    k(Zoom, "view.prevFrame", &["Shift+N", "Home"]),
    k(Zoom, "view.nextFrame", &["N", "End"]),
    // Text (on selected text layers, and on the selected characters while editing)
    k(Text, "text.bold", &["Cmd+B"]).both(),
    k(Text, "text.italic", &["Cmd+I"]).both(),
    k(Text, "text.underline", &["Cmd+U"]).both(),
    k(Text, "text.strikethrough", &["Shift+Cmd+X"]).both(),
    k(Text, "text.link", &["Shift+Cmd+U", "Cmd+K"]).text(),
    k(Text, "text.bulletList", &["Shift+Cmd+8"]).both(),
    k(Text, "text.numberList", &["Shift+Cmd+7"]).both(),
    k(Text, "text.listIndent", &["Cmd+]"]).text(),
    k(Text, "text.listOutdent", &["Cmd+["]).text(),
    k(Text, "text.alignLeft", &["Alt+Cmd+L"]).both(),
    k(Text, "text.alignCenter", &["Alt+Cmd+T"]).both(),
    k(Text, "text.alignRight", &["Alt+Cmd+R"]).both(),
    k(Text, "text.alignJustify", &["Alt+Cmd+J"]).both(),
    k(Text, "text.sizeUp", &["Shift+Cmd+."]).both(),
    k(Text, "text.sizeDown", &["Shift+Cmd+,"]).both(),
    k(Text, "text.weightUp", &["Alt+Cmd+."]).both(),
    k(Text, "text.weightDown", &["Alt+Cmd+,"]).both(),
    k(Text, "text.lineHeightUp", &["Alt+Shift+."]).both(),
    k(Text, "text.lineHeightDown", &["Alt+Shift+,"]).both(),
    k(Text, "text.trackingUp", &["Alt+."]).both(),
    k(Text, "text.trackingDown", &["Alt+,"]).both(),
    // Shape
    k(Shape, "vector.flatten", &["Alt+Shift+F", "Cmd+E"]),
    k(Shape, "vector.outlineStroke", &["Alt+Cmd+O"]),
    k(Shape, "bool.union", &["Alt+Shift+U"]),
    k(Shape, "bool.subtract", &["Alt+Shift+S"]),
    k(Shape, "bool.intersect", &["Alt+Shift+I"]),
    k(Shape, "bool.exclude", &["Alt+Shift+E"]),
    k(Shape, "object.removeFill", &["Alt+/"]),
    k(Shape, "object.removeStroke", &["Shift+/"]),
    k(Shape, "object.swapFillStroke", &["Shift+X"]),
    k(Shape, "vector.join", &["Cmd+J"]),
    k(Shape, "vector.smoothJoin", &["Shift+Cmd+J"]),
    k(Shape, "vector.deleteHeal", &["Shift+Delete"]),
    // Selection
    k(Selection, "select.all", &["Cmd+A"]),
    k(Selection, "select.inverse", &["Shift+Cmd+A"]),
    k(Selection, "select.matching", &["Alt+Cmd+A"]),
    k(Selection, "select.none", &["Escape"]),
    k(Selection, "select.children", &["Enter"]),
    k(Selection, "select.parent", &["Shift+Enter", "\\"]),
    k(Selection, "select.next", &["Tab"]),
    k(Selection, "select.previous", &["Shift+Tab"]),
    k(Selection, "object.group", &["Cmd+G"]),
    k(Selection, "object.ungroup", &["Shift+Cmd+G", "Cmd+Delete"]),
    k(Selection, "object.frameSelection", &["Alt+Cmd+G"]),
    k(Selection, "object.toggleVisible", &["Shift+Cmd+H"]),
    k(Selection, "object.toggleLock", &["Shift+Cmd+L"]),
    k(Selection, "object.rename", &["Cmd+R", "F2"]),
    // Alt+Space opens the window menu on Windows; the documented Windows key is Ctrl+Space.
    k(Selection, "select.area", &["Alt+Space", "F8"])
        .win(&["Ctrl+Space", "F8"])
        .label("Select with a box from the keyboard (arrows move it, Shift or Cmd/Ctrl+arrows size it, Enter selects)"),
    // Edit
    k(Edit, "edit.undo", &["Cmd+Z"]),
    k(Edit, "edit.redo", &["Shift+Cmd+Z"]),
    k(Edit, "edit.copy", &["Cmd+C"]),
    k(Edit, "edit.cut", &["Cmd+X"]),
    k(Edit, "edit.paste", &["Cmd+V"]),
    k(Edit, "edit.pasteOver", &["Shift+Cmd+V"]),
    k(Edit, "edit.pasteReplace", &["Shift+Cmd+R"]),
    k(Edit, "edit.duplicate", &["Cmd+D"]),
    k(Edit, "edit.delete", &["Delete"]),
    k(Edit, "edit.copyProperties", &["Alt+Cmd+C"]),
    k(Edit, "edit.pasteProperties", &["Alt+Cmd+V"]),
    k(Edit, "edit.copyAsPng", &["Shift+Cmd+C"]),
    k(Edit, "file.export", &["Shift+Cmd+E"]),
    k(Edit, "file.import", &["Shift+Cmd+I"]),
    // Transform
    k(Transform, "object.flipH", &["Shift+H"]),
    k(Transform, "object.flipV", &["Shift+V"]),
    k(Transform, "object.mask", &["Ctrl+Cmd+M"]).win(&["Ctrl+Alt+M"]),
    k(Transform, "object.resizeToFit", &["Alt+Shift+Cmd+R"]),
    k(Transform, "transform.rotationOrigin", &["Alt+R"]),
    // Arrange
    k(Arrange, "arrange.front", &["Alt+Cmd+]"]).win(&["Shift+Cmd+]"]),
    k(Arrange, "arrange.forward", &["Cmd+]"]),
    k(Arrange, "arrange.backward", &["Cmd+["]),
    k(Arrange, "arrange.back", &["Alt+Cmd+["]).win(&["Shift+Cmd+["]),
    k(Arrange, "align.left", &["Alt+A"]),
    k(Arrange, "align.hcenter", &["Alt+H"]),
    k(Arrange, "align.right", &["Alt+D"]),
    k(Arrange, "align.top", &["Alt+W"]),
    k(Arrange, "align.vcenter", &["Alt+V"]),
    k(Arrange, "align.bottom", &["Alt+S"]),
    k(Arrange, "distribute.horizontal", &["Ctrl+Alt+H"]).win(&["Alt+Shift+H"]),
    k(Arrange, "distribute.vertical", &["Ctrl+Alt+V"]).win(&["Alt+Shift+V"]),
    k(Arrange, "arrange.tidy", &["Ctrl+Alt+T"]).win(&["Ctrl+Alt+Shift+T"]),
    k(Arrange, "layout.add", &["Shift+A"]),
    k(Arrange, "layout.remove", &["Alt+Shift+A"]),
    k(Arrange, "layout.suggest", &["Ctrl+Shift+A"]).win(&["Ctrl+Alt+Shift+A"]),
    // Components
    k(Components, "component.create", &["Alt+Cmd+K"]),
    k(Components, "component.detach", &["Alt+Cmd+B"]),
    k(Components, "component.goToMain", &["Ctrl+Alt+Cmd+K"]).win(&["Ctrl+Alt+Shift+K"]),
    k(Components, "view.assets", &["Shift+I"]).label("Insert a component (Assets)"),
    // Prototype
    k(Prototype, "proto.present", &["Alt+Cmd+Enter"]).label("Present"),
    k(Prototype, "proto.preview", &["Shift+Space"]).label("Preview in the canvas"),
];

/// Pointer gestures and held keys the panel lists next to the bindings (handled by the canvas, not
/// dispatched from here): (group, label, macOS keys, Windows/Linux keys).
pub static GESTURES: &[(Group, &str, &str, &str)] = &[
    (Tools, "Pan the canvas", "Space drag", "Space+drag"),
    (Tools, "Pan with nothing selected (Shift: farther)", "← ↑ → ↓", "← ↑ → ↓"),
    (Tools, "Place a shape or frame (its tool chosen)", "↵", "Enter"),
    (Zoom, "Zoom tool: click to zoom in, drag an area", "Hold Z", "Hold Z"),
    (Zoom, "Zoom tool: zoom out", "Hold ⌥Z", "Hold Alt+Z"),
    (Zoom, "Zoom around the pointer", "⌘ scroll", "Ctrl+scroll"),
    (Selection, "Deep select", "⌘ click", "Ctrl+click"),
    (Selection, "Measure to the selection", "Hold ⌥", "Hold Alt"),
    (Edit, "Duplicate by dragging", "⌥ drag", "Alt+drag"),
    (Transform, "Nudge (Shift: big nudge)", "← ↑ → ↓", "← ↑ → ↓"),
    (Transform, "Resize by the nudge amount", "⌘ arrows", "Ctrl+arrows"),
    (Transform, "Opacity 10%…90%, 0: 100%", "1…9, 0", "1…9, 0"),
    (Transform, "Exact opacity (4 then 5: 45%)", "Two digits", "Two digits"),
    (Transform, "Suspend snapping while dragging", "Hold ⌃ or ⌘", "Hold Ctrl"),
    (Prototype, "Presenting: next frame", "→, Space or N", "→, Space or N"),
    (Prototype, "Presenting: previous frame", "←", "←"),
    (Prototype, "Presenting: restart", "R", "R"),
    (Prototype, "Presenting: cycle the size options", "Z", "Z"),
    (Prototype, "Presenting: comments", "C or ⇧C", "C or Shift+C"),
    (Prototype, "Presenting: full screen", "F", "F"),
    (View, "Next or previous control in the focused region", "⇥ / ⇧⇥", "Tab / Shift+Tab"),
    (View, "Move along the toolbar or the layer rows", "← → ↑ ↓", "← → ↑ ↓"),
    (View, "Back to the canvas from a focused region", "Esc", "Esc"),
];

/// Modifier keys held (as pressed: on Windows and Linux Ctrl may come as `cmd`, `ctrl` or both).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub cmd: bool,
    pub ctrl: bool,
}

impl Mods {
    pub fn is_none(self) -> bool {
        self == Mods::default()
    }
}

/// How far a text editing key moves or deletes on `platform`, as `(word, line)` for
/// [`crate::textedit::key`]. macOS: Alt by word, Cmd to the line's ends (Up/Down: the text's).
/// Windows and Linux: Ctrl by word, Ctrl+Home/End to the text's ends.
pub fn text_motion(platform: Platform, key: &str, m: Mods) -> (bool, bool) {
    match platform {
        Platform::Mac => (m.alt, m.cmd),
        Platform::Other if matches!(key, "Home" | "End") => (false, m.cmd || m.ctrl),
        Platform::Other => (m.cmd || m.ctrl, false),
    }
}

/// A key name as canonical (`=` for `Equals`, `ArrowLeft` for `Left`, `Delete` for `Backspace`),
/// and whether the name implies Shift (`?` is Shift+`/`). `None` for names that aren't keys.
pub fn normalize(name: &str) -> Option<(&str, bool)> {
    let shifted = |k: &'static str| Some((k, true));
    let plain = |k: &'static str| Some((k, false));
    match name {
        "" => None,
        "Plus" | "+" => shifted("="),
        "Questionmark" | "?" => shifted("/"),
        "<" => shifted(","),
        ">" => shifted("."),
        "OpenCurlyBracket" | "{" => shifted("["),
        "CloseCurlyBracket" | "}" => shifted("]"),
        "Pipe" | "|" => shifted("\\"),
        "Exclamationmark" | "!" => shifted("1"),
        "Colon" | ":" => shifted(";"),
        "\"" => shifted("'"),
        "_" => shifted("-"),
        "Equals" => plain("="),
        "Minus" => plain("-"),
        "OpenBracket" => plain("["),
        "CloseBracket" => plain("]"),
        "Backslash" => plain("\\"),
        "Quote" => plain("'"),
        "Comma" => plain(","),
        "Period" => plain("."),
        "Slash" => plain("/"),
        "Semicolon" => plain(";"),
        "Backtick" => plain("`"),
        "Backspace" | "Delete" | "Del" => plain("Delete"),
        "Return" | "Enter" => plain("Enter"),
        "Esc" | "Escape" => plain("Escape"),
        "Left" | "ArrowLeft" => plain("ArrowLeft"),
        "Right" | "ArrowRight" => plain("ArrowRight"),
        "Up" | "ArrowUp" => plain("ArrowUp"),
        "Down" | "ArrowDown" => plain("ArrowDown"),
        n if n.chars().count() == 1 => Some((n, false)),
        n if NAMED.iter().any(|k| k.eq_ignore_ascii_case(n)) => Some((n, false)),
        _ => None,
    }
}

const NAMED: &[&str] =
    &["Tab", "Space", "PageUp", "PageDown", "Home", "End", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "Menu"];

/// Parse a binding (`Shift+Cmd+K`) into its modifiers and canonical key.
pub fn parse(spec: &str) -> Option<(Mods, &str)> {
    let mut m = Mods::default();
    let mut key = None;
    for part in spec.split('+') {
        match part {
            "Shift" => m.shift = true,
            "Cmd" => m.cmd = true,
            "Alt" | "Option" => m.alt = true,
            "Ctrl" => m.ctrl = true,
            k => {
                if key.is_some() {
                    return None;
                }
                let (k, shift) = normalize(k)?;
                m.shift |= shift;
                key = Some(k);
            }
        }
    }
    key.map(|k| (m, k))
}

fn same_mods(platform: Platform, want: Mods, got: Mods) -> bool {
    match platform {
        Platform::Mac => want == got,
        // Cmd and Ctrl are one key here.
        Platform::Other => want.shift == got.shift && want.alt == got.alt && (want.cmd || want.ctrl) == (got.cmd || got.ctrl),
    }
}

/// Whether a binding fires for `key` with these modifiers on `platform`.
pub fn matches(spec: &str, platform: Platform, key: &str, mods: Mods) -> bool {
    let Some((key, shift)) = normalize(key) else { return false };
    let mods = Mods { shift: mods.shift || shift, ..mods };
    parse(spec).is_some_and(|(want, k)| k.eq_ignore_ascii_case(key) && same_mods(platform, want, mods))
}

/// The binding `key` with these modifiers fires in `context` on `platform`.
pub fn lookup(platform: Platform, context: Context, key: &str, mods: Mods) -> Option<&'static Binding> {
    KEYS.iter().filter(|b| b.live_in(context)).find(|b| b.keys(platform).iter().any(|s| matches(s, platform, key, mods)))
}

/// A command's keys on `platform` (all its bindings, the menu's first).
pub fn bindings(id: &str, platform: Platform) -> impl Iterator<Item = &'static str> {
    KEYS.iter().filter(move |b| b.id == id).flat_map(move |b| b.keys(platform).iter().copied())
}

/// The key a menu shows for a command on `platform`.
pub fn primary(id: &str, platform: Platform) -> Option<&'static str> {
    bindings(id, platform).next()
}

/// A binding as the platform writes it: `⇧⌘K` on macOS, `Ctrl+Shift+K` elsewhere.
pub fn display(spec: &str, platform: Platform) -> String {
    if spec.is_empty() {
        return String::new();
    }
    let mut m = Mods::default();
    let mut key = "";
    for part in spec.split('+') {
        match part {
            "Shift" => m.shift = true,
            "Cmd" => m.cmd = true,
            "Alt" | "Option" => m.alt = true,
            "Ctrl" => m.ctrl = true,
            k => key = k,
        }
    }
    // The zoom-in key reads as `+` (with Shift it types one).
    if key == "=" {
        key = "+";
        m.shift = false;
    }
    let mac = platform == Platform::Mac;
    let key = match (key, mac) {
        ("Delete", true) => "⌫",
        ("Enter", true) => "↵",
        ("Tab", true) => "⇥",
        ("PageUp", true) => "⇞",
        ("PageDown", true) => "⇟",
        ("Delete", false) => "Backspace",
        ("PageUp", false) => "PgUp",
        ("PageDown", false) => "PgDn",
        ("Escape", _) => "Esc",
        ("ArrowLeft", _) => "←",
        ("ArrowRight", _) => "→",
        ("ArrowUp", _) => "↑",
        ("ArrowDown", _) => "↓",
        (k, _) => k,
    };
    let mut out = String::new();
    if mac {
        for (on, glyph) in [(m.ctrl, "⌃"), (m.alt, "⌥"), (m.shift, "⇧"), (m.cmd, "⌘")] {
            if on {
                out.push_str(glyph);
            }
        }
        out.push_str(key);
    } else {
        for (on, name) in [(m.ctrl || m.cmd, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (!key.is_empty(), key)] {
            if on {
                if !out.is_empty() {
                    out.push('+');
                }
                out.push_str(name);
            }
        }
    }
    out
}
