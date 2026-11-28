use base64::Engine as _;

use super::css::{Tokens, css_decls_for_markup};
use super::{CodeLang, Ctx, Document, LayoutMode, Node, NodeId, NodeKind, Param, Sizing, indent, num, with_v_if};

// ------------------------------------------------------------------ HTML and React markup

/// A word for the node's kind, naming layers whose name transliterates to nothing ("###" →
/// `.text`, `.frame`), instead of the anonymous `layer-`.
fn kind_word(n: &Node) -> &str {
    match &n.kind {
        NodeKind::Page { .. } => "page",
        NodeKind::Frame => "frame",
        NodeKind::Group => "group",
        NodeKind::Section { .. } => "section",
        NodeKind::Rectangle => "rectangle",
        NodeKind::Ellipse { .. } => "ellipse",
        NodeKind::Polygon { .. } => "polygon",
        NodeKind::Star { .. } => "star",
        NodeKind::Line => "line",
        NodeKind::Vector { .. } => "vector",
        NodeKind::Text(_) => "text",
        NodeKind::BooleanOp { .. } => "boolean",
        NodeKind::Component { .. } | NodeKind::ComponentSet { .. } => "component",
        NodeKind::Instance { .. } => "instance",
        NodeKind::Slice => "slice",
    }
}

/// A CSS class name from a layer name ("Card / Title" → "card-title"), unique in `used`. The name
/// is transliterated to ASCII first, so non-Latin names stay readable: 로그인 화면 →
/// `rogeuin-hwamyeon`, кнопка → `knopka`, über → `uber` (per-codepoint transliteration, the slug
/// libraries' way); a name left with nothing (symbols only) takes `kind`, and one starting with a
/// digit keeps the `layer-` prefix.
pub(crate) fn class_name(name: &str, kind: &str, used: &mut Names) -> String {
    let mut base = String::new();
    for c in deunicode::deunicode(name).chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            base.push(c);
        } else if !base.ends_with('-') && !base.is_empty() {
            base.push('-');
        }
    }
    // The 40-character cut lands on a word boundary when there is one past the first word.
    let base = if base.chars().count() > 40 {
        let cut: String = base.chars().take(40).collect();
        match cut.rfind('-') {
            Some(i) if i > 0 => cut[..i].to_string(),
            _ => cut,
        }
    } else {
        base
    };
    let base = base.trim_end_matches('-');
    let base = if base.is_empty() {
        kind.to_string()
    } else if base.starts_with(|c: char| c.is_ascii_digit()) {
        format!("layer-{base}")
    } else {
        base.to_string()
    };
    let from = used.next.get(&base).copied().unwrap_or(1);
    let (k, name) = (from..)
        .map(|k| (k, if k == 1 { base.clone() } else { format!("{base}-{k}") }))
        .find(|(_, n)| !used.used.contains(n))
        .unwrap_or((from, base.clone()));
    used.next.insert(base, k + 1);
    used.insert(name.clone());
    name
}

/// The names [`class_name`] has given out, and the next suffix to try per base, so many layers
/// sharing one name don't each retry every suffix from 2 (that was cubic in their number).
#[derive(Default)]
pub(crate) struct Names {
    used: std::collections::BTreeSet<String>,
    next: std::collections::BTreeMap<String, u64>,
}

impl Names {
    pub fn insert(&mut self, name: String) {
        self.used.insert(name);
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\n', "<br>")
}

/// A layer name in a quoted markup attribute, with line breaks flattened.
fn attr_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace(['\n', '\r'], " ")
}

/// A vector as a self-contained SVG asset and its local painted bounds. CSS owns the node's
/// placement and opacity, so the embedded geometry is node-local and fully opaque. No filesystem
/// export or URL escaping is needed, and nested vectors work exactly like top-level ones.
fn vector_data_uri(doc: &mut Document, n: &Node) -> Option<(String, figcraft_geom::Rect)> {
    let node = doc.get_mut(n.id).ok()?;
    node.parent = None;
    node.transform = figcraft_geom::Affine::IDENTITY;
    node.opacity = 1.0;
    let bounds = figcraft_render::export_bounds(doc, n.id)?;
    let svg = figcraft_svg::export_svg(doc, &[n.id], &Default::default())?;
    Some((format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg)), bounds))
}

/// Text as JSX children: HTML escapes, braces as expressions, line breaks as `<br />`.
fn jsx_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().take(10_000) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '{' => out.push_str("{\"{\"}"),
            '}' => out.push_str("{\"}\"}"),
            '\n' => out.push_str("<br />"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// The whole tree under `id` as nested HTML with a stylesheet: auto layout frames become
/// flexboxes, other children are placed absolutely in their parent, text becomes paragraphs.
pub(super) fn html(doc: &Document, id: NodeId) -> String {
    let ctx = Ctx::new(doc, id);
    let mut out = Markup::new(&ctx, CodeLang::Html);
    out.node(id, None, 0, "");
    // Document text can reach the stylesheet through any property: `<\/` reads back as `</` inside a CSS
    // string and can never close the element early.
    let css = format!("{}{}", out.tokens.root_rule(), out.styles).replace("</", "<\\/");
    format!("{}\n<style>\n{css}</style>\n", out.markup)
}

/// Markup being built for a layer tree: HTML (`class`), React JSX (`className`), Vue
/// (`{{ }}` interpolation, `v-if`, `:attr` bindings) or Leptos `view!` markup (text as Rust
/// string literals); the last three with component parameters and instances as calls. With its
/// stylesheet.
pub(crate) struct Markup<'c, 'a> {
    ctx: &'c Ctx<'a>,
    /// Html, React, Vue or Leptos.
    lang: CodeLang,
    used: Names,
    pub markup: String,
    pub styles: String,
    pub tokens: Tokens,
    /// The root's class (default: from its name) and extra JSX attributes.
    pub root_class: Option<String>,
    pub root_attrs: String,
    /// A Vue `v-if` guard for the element about to be written: Vue templates can't wrap an
    /// element in an expression, so a boolean parameter's layer carries the condition itself.
    v_if: Option<String>,
    /// A prefix for every generated class name (slot content joins the caller's stylesheet, where
    /// two slots' layer names may repeat).
    pub class_prefix: Option<String>,
    /// Each layer's path in the tree (names with sibling counts), class and declarations, so
    /// variants can be compared layer by layer.
    pub rules: Vec<(String, String, String)>,
    /// Instances as their own (overridden) layers instead of component calls, except those
    /// connected to code (page code).
    pub inline: bool,
    /// Auto layout frames and text as fluid boxes (`width: 100%` up to their width) for pages.
    pub fluid: bool,
    /// With `fluid`, other layers at least this share of the root's width are fluid too (a
    /// divider, a banner image); smaller ones (icons) keep their size. Default: none.
    pub fluid_leaves: f64,
    /// Lazily cloned once for node-local SVG assets, not once per vector.
    asset_doc: Option<Document>,
}

/// How `n` sits in its parent: in the flex flow, or placed at its position.
fn placement_decls(n: &Node, parent: Option<&Node>, root: bool, in_flow: bool) -> String {
    let mut decls = String::new();
    if root {
        decls.push_str("position: relative;\n");
    } else if in_flow {
        let vertical = parent.and_then(|p| p.layout.as_ref()).is_some_and(|l| l.mode == LayoutMode::Vertical);
        let (main, cross) =
            if vertical { (n.child_layout.sizing_v, n.child_layout.sizing_h) } else { (n.child_layout.sizing_h, n.child_layout.sizing_v) };
        if main == Sizing::Fill {
            decls.push_str("flex: 1 1 0;\n");
        }
        if cross == Sizing::Fill {
            decls.push_str("align-self: stretch;\n");
        }
    } else {
        let c = n.transform.as_coeffs();
        decls.push_str(&format!("position: absolute;\nleft: {}px;\ntop: {}px;\n", num(c[4]), num(c[5])));
        let rot = figcraft_geom::rotation_degrees(n.transform);
        if rot.abs() > 0.01 {
            decls.push_str(&format!("transform: rotate({}deg);\ntransform-origin: 0 0;\n", num(-rot)));
        }
    }
    decls
}

/// Extra declarations around an embedded vector image. The inner SVG can be larger than the
/// node box, so the wrapper must establish positioning in flex flow and leave overflow visible.
fn vector_wrapper_decls(n: &Node, in_flow: bool, own: &mut String) {
    if !matches!(n.kind, NodeKind::Vector { .. }) {
        return;
    }
    own.push_str("overflow: visible;\n");
    if in_flow {
        own.push_str("position: relative;\n");
    }
}

impl<'c, 'a> Markup<'c, 'a> {
    pub fn new(ctx: &'c Ctx<'a>, lang: CodeLang) -> Self {
        Markup {
            ctx,
            lang,
            used: Names::default(),
            markup: String::new(),
            styles: String::new(),
            tokens: Tokens::default(),
            root_class: None,
            root_attrs: String::new(),
            v_if: None,
            class_prefix: None,
            rules: Vec::new(),
            inline: false,
            fluid: false,
            fluid_leaves: f64::INFINITY,
            asset_doc: None,
        }
    }

    /// JSX and Vue templates indent by two spaces, HTML and Leptos by four.
    fn pad(&self, depth: usize) -> String {
        if matches!(self.lang, CodeLang::React | CodeLang::Vue) { "  ".repeat(depth) } else { indent(depth) }
    }

    /// Component code (React or Leptos): parameters and instances as calls.
    fn code(&self) -> bool {
        self.lang != CodeLang::Html
    }

    /// A parameter's name in the generated code.
    fn ident(&self, p: &Param) -> String {
        if self.lang == CodeLang::Leptos { crate::component_codegen::rust_ident(&p.ident) } else { p.ident.clone() }
    }

    /// A layer, shown only while its boolean parameter is on (component code).
    pub fn node(&mut self, id: NodeId, parent: Option<&Node>, depth: usize, path: &str) {
        let Some(n) = self.ctx.doc.node(id) else { return };
        if self.code()
            && id != self.ctx.root
            && let Some(p) = self.ctx.param(n, "visible")
        {
            // A Vue template holds no arbitrary expressions, so the guard rides on the element.
            if self.lang == CodeLang::Vue {
                self.v_if = Some(self.ident(p));
                self.element(n, parent, depth, path);
                self.v_if = None;
                return;
            }
            let (pad, p) = (self.pad(depth), self.ident(p));
            let (open, close) =
                if self.lang == CodeLang::Leptos { (format!("{{{p}.then(|| view! {{"), "})}") } else { (format!("{{{p} && ("), ")}") };
            self.markup.push_str(&format!("{pad}{open}\n"));
            self.element(n, parent, depth + 1, path);
            self.markup.push_str(&format!("{pad}{close}\n"));
            return;
        }
        self.element(n, parent, depth, path);
    }

    fn rule(&mut self, path: &str, class: &str, decls: String) {
        self.styles.push_str(&format!(".{class} {{\n"));
        for line in decls.lines() {
            self.styles.push_str(&format!("  {line}\n"));
        }
        self.styles.push_str("}\n");
        self.rules.push((path.to_string(), class.to_string(), decls));
    }

    /// An instance as a call to its component (see `element`); true when it was written.
    fn instance_call(&mut self, n: &Node, depth: usize, path: &str, class: &str, decls: &str, root: bool) -> bool {
        let pad = self.pad(depth);
        let attr = if self.lang == CodeLang::React { "className" } else { "class" };
        let connected = self.inline && crate::code_connect::link(self.ctx.doc, n.id, CodeLang::React).is_some();
        if self.code() && (!self.inline || connected) && !root && matches!(n.kind, NodeKind::Instance { .. }) {
            let at = depth + usize::from(!decls.is_empty());
            let run = if self.lang == CodeLang::Leptos { ".run()" } else { "" };
            let (inner, css) = match self.ctx.param(n, "mainComponent") {
                Some(p) if self.lang == CodeLang::Vue => (Some(format!("{}<component :is=\"{}\" />\n", self.pad(at), self.ident(p))), String::new()),
                Some(p) => (Some(format!("{}{{{}{run}}}\n", self.pad(at), self.ident(p))), String::new()),
                None => self.ctx.call(n, self.lang, at).map_or((None, String::new()), |(code, css)| (Some(code), css)),
            };
            self.styles.push_str(&css);
            if let Some(inner) = inner {
                // A Vue guard set by `node` rides on the call when there is no wrapper.
                let guard = if self.lang == CodeLang::Vue { self.v_if.take() } else { None };
                if decls.is_empty() {
                    self.markup.push_str(&with_v_if(&inner, guard.as_deref()));
                } else {
                    let guard = guard.map(|g| format!(" v-if=\"{g}\"")).unwrap_or_default();
                    self.markup.push_str(&format!("{pad}<div {attr}=\"{class}\"{guard}>\n{inner}{pad}</div>\n"));
                    self.rule(path, class, decls.to_string());
                }
                return true;
            }
        }
        false
    }

    fn text_body(&self, n: &Node, t: &figcraft_doc::TextData) -> String {
        match (self.ctx.param(n, "characters").filter(|_| self.code()), self.lang) {
            (Some(p), CodeLang::Leptos) => format!("{{{}.clone()}}", self.ident(p)),
            (Some(p), CodeLang::Vue) => format!("{{{{ {} }}}}", self.ident(p)),
            (Some(p), _) => format!("{{{}}}", p.ident),
            (None, CodeLang::React) => jsx_text(&t.characters),
            // `view!` takes text as string literals and void elements self-closed.
            (None, CodeLang::Leptos) => {
                t.characters.chars().take(10_000).collect::<String>().lines().map(|l| format!("{l:?}")).collect::<Vec<_>>().join("<br/>")
            }
            // A Vue template reads `{{ }}` as interpolation, so braces are escaped.
            (None, CodeLang::Vue) => html_escape(&t.characters).replace('{', "&#123;").replace('}', "&#125;"),
            (None, _) => html_escape(&t.characters),
        }
    }

    /// A node-sized wrapper and its inner SVG image. The inner rule uses the exported bounds,
    /// so strokes and effects keep their size and can overflow the wrapper without clipping.
    fn vector_element(&mut self, n: &Node, depth: usize, path: &str, class: &str, attr: &str, pad: &str, attrs: &str) {
        let root_doc = self.ctx.doc;
        let asset = vector_data_uri(self.asset_doc.get_or_insert_with(|| root_doc.clone()), n);
        let bounds = asset.as_ref().map(|(_, b)| *b).unwrap_or_else(|| n.local_rect());
        let asset_class = class_name(&format!("{class} asset"), "asset", &mut self.used);
        let asset_decls = format!(
            "display: block;\nposition: absolute;\nleft: {}px;\ntop: {}px;\nwidth: {}px;\nheight: {}px;\nmax-width: none;\n",
            num(bounds.x0),
            num(bounds.y0),
            num(bounds.width()),
            num(bounds.height())
        );
        self.rule(&format!("{path}/$svg"), &asset_class, asset_decls);
        // Leptos attributes are Rust string literals; the other targets use HTML escaping.
        let lang = self.lang;
        let quote = |value: &str| if lang == CodeLang::Leptos { format!("{value:?}") } else { format!("\"{}\"", attr_escape(value)) };
        let src = asset.map(|(uri, _)| format!(" src={}", quote(&uri))).unwrap_or_default();
        let alt = quote(&n.name);
        let inner = self.pad(depth + 1);
        self.markup.push_str(&format!("{pad}<div {attr}=\"{class}\"{attrs}>\n{inner}<img {attr}=\"{asset_class}\"{src} alt={alt} />\n{pad}</div>\n"));
    }

    fn element(&mut self, n: &Node, parent: Option<&Node>, depth: usize, path: &str) {
        let doc = self.ctx.doc;
        let root = n.id == self.ctx.root;
        let class = match self.root_class.take().filter(|_| root) {
            Some(c) => {
                self.used.insert(c.clone());
                c
            }
            None => {
                let class = class_name(&n.name, kind_word(n), &mut self.used);
                match &self.class_prefix {
                    Some(prefix) => format!("{prefix}{class}"),
                    None => class,
                }
            }
        };
        let pad = self.pad(depth);
        let attr = if self.lang == CodeLang::React { "className" } else { "class" };
        // How it sits in its parent: in the flex flow, or placed at its position.
        let in_flow = parent.is_some_and(|p| p.layout.is_some()) && !n.child_layout.absolute;
        let mut decls = placement_decls(n, parent, root, in_flow);
        // Nested instances are calls (component code), in a wrapper that places them when needed.
        // Page code inlines instances, except those connected to code (codeConnect.set): their
        // snippet, e.g. a design-system `<Button>`.
        if self.instance_call(n, depth, path, &class, &decls, root) {
            return;
        }
        let hugs = n.layout.is_some() || matches!(n.kind, NodeKind::Text(_));
        let hug = (hugs && n.child_layout.sizing_h == Sizing::Hug, hugs && n.child_layout.sizing_v == Sizing::Hug);
        let mut own = css_decls_for_markup(doc, n, &mut self.tokens, hug);
        let page = self.ctx.doc.node(self.ctx.root).map_or(f64::INFINITY, |r| r.width);
        if self.fluid && (n.layout.is_some() || matches!(n.kind, NodeKind::Text(_)) || (!root && n.width >= page * self.fluid_leaves)) {
            // Full width, or sized by the flex row (`flex: 1`): no cap needed.
            own = fluid(&own, n.width >= page - 0.5 || decls.contains("flex: 1 1 0;"), n.layout.is_some());
        }
        if n.layout.is_none() && !n.children.is_empty() && !root && in_flow {
            own.push_str("position: relative;\n");
        }
        vector_wrapper_decls(n, in_flow, &mut own);
        decls.push_str(&own);
        self.rule(path, &class, decls);
        // The root's variant attributes, and a Vue `v-if` guard, ride on the open tag.
        let mut attrs = if root { self.root_attrs.clone() } else { String::new() };
        if self.lang == CodeLang::Vue
            && let Some(guard) = self.v_if.take()
        {
            attrs.push_str(&format!(" v-if=\"{guard}\""));
        }
        let open = |tag: &str| format!("{pad}<{tag} {attr}=\"{class}\"{attrs}>");
        match &n.kind {
            NodeKind::Vector { .. } => self.vector_element(n, depth, path, &class, attr, &pad, &attrs),
            NodeKind::Text(t) => {
                let body = self.text_body(n, t);
                self.markup.push_str(&format!("{}{body}</p>\n", open("p")));
            }
            _ => {
                let kids = self.ctx.kids(n);
                let slot = self.ctx.param(n, "slot").filter(|_| self.code());
                let vue = self.lang == CodeLang::Vue;
                let leptos = self.lang == CodeLang::Leptos;
                let open = open("div");
                let slot_ident = slot.map(|p| match self.ctx.comp {
                    Some(c) => crate::component_codegen::slot_name(c, p),
                    None => class_name(&p.ident, "slot", &mut Names::default()),
                });
                if let Some(p) = slot.filter(|_| kids.is_empty()) {
                    let content = if leptos {
                        format!("{{{}}}.as_ref().map(|c| c.run())", self.ident(p))
                    } else if vue {
                        // Vue's own slot element, named so an instance's content lands in the right one.
                        match slot_ident.clone() {
                            Some(name) => format!("<slot name=\"{name}\" />"),
                            None => "<slot />".into(),
                        }
                    } else {
                        format!("{{{}}}", p.ident)
                    };
                    self.markup.push_str(&format!("{open}{content}</div>\n"));
                } else if kids.is_empty() || self.ctx.too_deep(depth) {
                    self.markup.push_str(&format!("{open}</div>\n"));
                } else {
                    self.markup.push_str(&format!("{open}\n"));
                    // A slot shows the instance's content, or else the main's.
                    let (p1, p2) = (self.pad(depth + 1), self.pad(depth + 2));
                    let inner = match slot {
                        Some(_) if vue => {
                            let name = slot_ident.clone().map(|n| format!(" name=\"{n}\"")).unwrap_or_default();
                            self.markup.push_str(&format!("{p1}<slot{name}>\n"));
                            depth + 2
                        }
                        Some(p) if leptos => {
                            self.markup.push_str(&format!("{p1}{{match &{} {{ Some(c) => c.run(), None => view! {{\n", self.ident(p)));
                            depth + 2
                        }
                        Some(p) => {
                            self.markup.push_str(&format!("{p1}{{{} ?? (\n{p2}<>\n", p.ident));
                            depth + 3
                        }
                        None => depth + 1,
                    };
                    let mut seen: Vec<&str> = Vec::new();
                    // Bottom layer first: later elements paint on top, like the layer order.
                    for k in kids {
                        let Some(kn) = doc.node(k) else { continue };
                        let count = seen.iter().filter(|s| **s == kn.name).count();
                        seen.push(&kn.name);
                        self.node(k, Some(n), inner, &format!("{path}/{}#{count}", kn.name));
                    }
                    match slot {
                        Some(_) if vue => self.markup.push_str(&format!("{p1}</slot>\n")),
                        Some(_) if leptos => self.markup.push_str(&format!("{p1}}}.into_any() }}}}\n")),
                        Some(_) => self.markup.push_str(&format!("{p2}</>\n{p1})}}\n")),
                        None => {}
                    }
                    self.markup.push_str(&format!("{pad}</div>\n"));
                }
            }
        }
    }
}

/// `decls` for a box that follows the page: a fixed width becomes `width: 100%` capped at that
/// width (or just the full width for boxes as wide as the page), and a frame's fixed height a
/// minimum, so content can wrap and grow.
// ponytail: width-based heuristic; fixed-size leaves (icons, images, shapes) keep their pixels.
fn fluid(decls: &str, full: bool, frame: bool) -> String {
    let mut out = String::with_capacity(decls.len() + 32);
    for line in decls.lines() {
        if let Some(w) = line.strip_prefix("width: ") {
            out.push_str("width: 100%;\n");
            if !full {
                out.push_str(&format!("max-width: {w}\n"));
            }
        } else if let Some(h) = line.strip_prefix("height: ").filter(|_| frame) {
            out.push_str(&format!("min-height: {h}\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
