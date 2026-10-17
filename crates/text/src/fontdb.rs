//! The font database: embedded craft-fonts faces plus (on desktop) the system's fonts.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use figcraft_geom::BezPath;
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::{FileRef, TableProvider};
use skrifa::string::StringId;
use skrifa::{GlyphId, MetadataProvider};

use crate::color::{ColorLayer, Rasters};

/// A font from the optional craft-fonts build input (empty unless built with `CRAFT_FONTS_DIR`).
pub struct CraftFont {
    pub family: &'static str,
    pub style: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

#[derive(Clone)]
pub enum FontBytes {
    Static(&'static [u8]),
    Shared(Arc<Vec<u8>>),
}

impl FontBytes {
    pub fn data(&self) -> &[u8] {
        match self {
            FontBytes::Static(b) => b,
            FontBytes::Shared(v) => v.as_slice(),
        }
    }
}

static NEXT_FACE: AtomicU32 = AtomicU32::new(1);

/// One variation axis of a variable font (user-space values).
#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    /// The 4-char OpenType tag (`wght`, `wdth`, `slnt`, `ital`, `opsz`, or a custom one).
    pub tag: String,
    /// The axis name from the font (falls back to a readable name for registered tags).
    pub name: String,
    pub min: f64,
    pub default: f64,
    pub max: f64,
    /// The font asks apps not to show this axis.
    pub hidden: bool,
}

impl Axis {
    pub fn clamp(&self, v: f64) -> f64 {
        if v.is_finite() { v.clamp(self.min, self.max) } else { self.default }
    }
}

/// A named instance of a variable font ("Semi Bold", "Condensed Light"…): one user value per axis.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedInstance {
    pub name: String,
    pub coords: Vec<f64>,
}

/// Normalized variation coordinates to draw a glyph with (empty = the font's default instance).
pub type Coords = Arc<[NormalizedCoord]>;

/// Readable names for the registered axes (used when the font doesn't name them).
pub fn axis_label(tag: &str) -> &str {
    match tag {
        "wght" => "Weight",
        "wdth" => "Width",
        "slnt" => "Slant",
        "ital" => "Italic",
        "opsz" => "Optical size",
        other => other,
    }
}

/// One loaded font face.
pub struct Face {
    pub id: u32,
    pub family: String,
    pub style: String,
    pub weight: u16,
    pub italic: bool,
    pub bytes: FontBytes,
    pub index: u32,
    pub upem: f64,
    /// Ascent and descent in font units, both positive.
    pub ascent: f64,
    pub descent: f64,
    pub line_gap: f64,
    pub cap_height: f64,
    pub x_height: f64,
    pub underline_pos: f64,
    pub underline_thickness: f64,
    pub strike_pos: f64,
    pub shaper: harfrust::ShaperData,
    /// OpenType GSUB feature tags the face provides (e.g. `smcp`).
    gsub_features: Vec<[u8; 4]>,
    /// Variation axes (empty for static fonts) and named instances.
    pub axes: Vec<Axis>,
    pub instances: Vec<NamedInstance>,
    charmap: HashMap<u32, u32>,
    outlines: Mutex<HashMap<(u32, Coords), Arc<BezPath>>>,
    advances: Mutex<HashMap<(u32, Coords), f64>>,
    /// Colour glyph tables (see [`crate::color`]): bitmap strikes and/or COLR layers.
    pub(crate) bitmap_strikes: bool,
    pub(crate) colr: bool,
    pub(crate) rasters: Mutex<Rasters>,
    pub(crate) layers: Mutex<HashMap<u32, Option<Arc<Vec<ColorLayer>>>>>,
}

impl std::fmt::Debug for Face {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Face({} {} #{})", self.family, self.style, self.id)
    }
}

impl Face {
    pub fn from_bytes(bytes: FontBytes, index: u32, family_override: Option<&str>, style_override: Option<&str>) -> Option<Face> {
        let data = bytes.data();
        let f = skrifa::FontRef::from_index(data, index).ok()?;
        let (family, style) = names(&f);
        let family = family_override.map(str::to_string).or(family)?;
        let style = style_override.map(str::to_string).or(style).unwrap_or_else(|| "Regular".into());
        let m = f.metrics(Size::unscaled(), LocationRef::default());
        let attrs = f.attributes();
        let hb = harfrust::FontRef::from_index(data, index).ok()?;
        let shaper = harfrust::ShaperData::new(&hb);
        let upem = f64::from(m.units_per_em.max(1));
        let ascent = f64::from(m.ascent);
        let descent = -f64::from(m.descent);
        let mut gsub_features: Vec<[u8; 4]> = f
            .gsub()
            .ok()
            .and_then(|g| g.feature_list().ok())
            .map(|l| l.feature_records().iter().map(|r| r.feature_tag().to_be_bytes()).collect())
            .unwrap_or_default();
        gsub_features.sort_unstable();
        gsub_features.dedup();
        let charmap: HashMap<u32, u32> = f.charmap().mappings().map(|(c, g)| (c, g.to_u32())).collect();
        let (axes, instances) = variation_info(&f);
        let has_table = |tag: &[u8; 4]| f.data_for_tag(skrifa::Tag::new(tag)).is_some();
        let bitmap_strikes = has_table(b"sbix") || has_table(b"CBDT") || has_table(b"EBDT");
        let colr = has_table(b"COLR");
        let under = m.underline.map(|d| (f64::from(d.offset), f64::from(d.thickness))).unwrap_or((-upem * 0.1, upem * 0.06));
        let strike = m.strikeout.map(|d| f64::from(d.offset)).unwrap_or(ascent * 0.3);
        let weight = {
            let w = attrs.weight.value();
            if w.is_finite() && w > 0.0 { w.round() as u16 } else { figcraft_doc::style_weight(&style) }
        };
        Some(Face {
            id: NEXT_FACE.fetch_add(1, Ordering::Relaxed),
            italic: !matches!(attrs.style, skrifa::attribute::Style::Normal) || style.to_ascii_lowercase().contains("italic"),
            weight,
            family,
            style,
            index,
            upem,
            ascent,
            descent,
            line_gap: f64::from(m.leading),
            cap_height: m.cap_height.map(f64::from).filter(|v| *v > 0.0).unwrap_or(ascent * 0.72),
            x_height: m.x_height.map(f64::from).filter(|v| *v > 0.0).unwrap_or(ascent * 0.5),
            underline_pos: under.0,
            underline_thickness: under.1.max(upem * 0.02),
            strike_pos: strike,
            shaper,
            gsub_features,
            axes,
            instances,
            charmap,
            bytes,
            outlines: Mutex::new(HashMap::new()),
            advances: Mutex::new(HashMap::new()),
            bitmap_strikes,
            colr,
            rasters: Mutex::new(Rasters::default()),
            layers: Mutex::new(HashMap::new()),
        })
    }

    /// Whether the face's GSUB table has feature `tag` (e.g. `b"smcp"`).
    pub fn has_feature(&self, tag: &[u8; 4]) -> bool {
        self.gsub_features.binary_search(tag).is_ok()
    }

    /// The face's GSUB feature tags.
    pub fn feature_tags(&self) -> Vec<String> {
        self.gsub_features.iter().map(|t| String::from_utf8_lossy(t).into_owned()).collect()
    }

    pub fn has_char(&self, c: char) -> bool {
        self.charmap.get(&(c as u32)).is_some_and(|g| *g != 0)
    }

    pub fn glyph_for(&self, c: char) -> u32 {
        self.charmap.get(&(c as u32)).copied().unwrap_or(0)
    }

    pub fn hb(&self) -> Option<harfrust::FontRef<'_>> {
        harfrust::FontRef::from_index(self.bytes.data(), self.index).ok()
    }

    /// Glyph outline in font units, y down, at the default instance.
    pub fn outline(&self, gid: u32) -> Arc<BezPath> {
        self.outline_at(gid, &no_coords())
    }

    /// Glyph outline in font units, y down, at variation `coords`.
    pub fn outline_at(&self, gid: u32, coords: &Coords) -> Arc<BezPath> {
        let key = (gid, coords.clone());
        if let Some(p) = self.outlines.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
            return p.clone();
        }
        let mut pen = FlipPen(BezPath::new());
        if let Ok(f) = skrifa::FontRef::from_index(self.bytes.data(), self.index)
            && let Some(g) = f.outline_glyphs().get(GlyphId::new(gid))
        {
            let _ = g.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::new(coords)), &mut pen);
        }
        let p = Arc::new(pen.0);
        let mut cache = self.outlines.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() > 20_000 {
            cache.clear();
        }
        cache.insert(key, p.clone());
        p
    }

    /// Horizontal advance in font units at the default instance.
    pub fn advance(&self, gid: u32) -> f64 {
        self.advance_at(gid, &no_coords())
    }

    /// Horizontal advance in font units at variation `coords`.
    pub fn advance_at(&self, gid: u32, coords: &Coords) -> f64 {
        let key = (gid, coords.clone());
        if let Some(a) = self.advances.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
            return *a;
        }
        let adv = skrifa::FontRef::from_index(self.bytes.data(), self.index)
            .ok()
            .and_then(|f| f.glyph_metrics(Size::unscaled(), LocationRef::new(coords)).advance_width(GlyphId::new(gid)))
            .map(f64::from)
            .filter(|a| a.is_finite())
            .unwrap_or(self.upem * 0.5);
        let mut cache = self.advances.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() > 50_000 {
            cache.clear();
        }
        cache.insert(key, adv);
        adv
    }

    /// Whether the face is a variable font.
    pub fn is_variable(&self) -> bool {
        !self.axes.is_empty()
    }

    pub fn axis(&self, tag: &str) -> Option<&Axis> {
        self.axes.iter().find(|a| a.tag == tag)
    }

    /// The named instance called `style` (case, spaces and hyphens ignored).
    pub fn instance(&self, style: &str) -> Option<&NamedInstance> {
        let want = norm_style(style);
        self.instances.iter().find(|i| norm_style(&i.name) == want)
    }

    /// The user-space axis values `style` asks of this face, one per axis in axis order (empty for
    /// static fonts). Starts from the named instance matching the style name (else the weight and
    /// italic the name implies, with an optical size that follows the font size), then applies the
    /// style's custom values; every value is clamped to its axis.
    pub fn axis_values(&self, style: &figcraft_doc::TextStyle) -> Vec<f64> {
        if self.axes.is_empty() {
            return Vec::new();
        }
        let mut v: Vec<f64> = self.axes.iter().map(|a| a.default).collect();
        if let Some(inst) = self.instance(&style.font_style) {
            for (slot, c) in v.iter_mut().zip(&inst.coords) {
                *slot = *c;
            }
        } else {
            let own_style = norm_style(&self.style) == norm_style(&style.font_style);
            let italic = style.font_style.to_ascii_lowercase().contains("italic") && !self.italic;
            for (slot, a) in v.iter_mut().zip(&self.axes) {
                match a.tag.as_str() {
                    "wght" if !own_style => *slot = f64::from(figcraft_doc::style_weight(&style.font_style)),
                    "ital" if italic => *slot = 1.0,
                    "slnt" if italic => *slot = a.min,
                    "opsz" if style.font_size.is_finite() => *slot = style.font_size,
                    _ => {}
                }
            }
        }
        for (slot, a) in v.iter_mut().zip(&self.axes) {
            if let Some(x) = style.variations.get(&a.tag) {
                *slot = *x;
            }
            *slot = a.clamp(*slot);
        }
        v
    }

    /// Normalized coordinates for user-space `values` (one per axis; empty = default instance).
    pub fn coords(&self, values: &[f64]) -> Coords {
        if self.axes.is_empty() || values.len() != self.axes.len() {
            return no_coords();
        }
        let Ok(f) = skrifa::FontRef::from_index(self.bytes.data(), self.index) else { return no_coords() };
        let settings: Vec<(skrifa::Tag, f32)> = self.axes.iter().zip(values).filter_map(|(a, v)| Some((tag_of(&a.tag)?, *v as f32))).collect();
        let loc = f.axes().location(settings);
        let c = loc.coords();
        if c.iter().all(|x| *x == NormalizedCoord::ZERO) { no_coords() } else { Coords::from(c) }
    }
}

/// The default instance's coordinates.
pub fn no_coords() -> Coords {
    Coords::from([])
}

fn tag_of(s: &str) -> Option<skrifa::Tag> {
    <[u8; 4]>::try_from(s.as_bytes()).ok().map(|b| skrifa::Tag::new(&b))
}

fn norm_style(s: &str) -> String {
    s.to_lowercase().replace([' ', '-', '_'], "")
}

/// A style name's weight before its file is read: the usual names, and Hiragino's "W3" (300).
fn pending_weight(style: &str) -> i32 {
    let w = norm_style(style).strip_prefix('w').and_then(|n| n.parse::<i32>().ok()).filter(|n| (0..=9).contains(n));
    w.map_or(i32::from(figcraft_doc::style_weight(style)), |n| n * 100)
}

/// The axes (user-space ranges) and named instances of a variable font.
fn variation_info(f: &skrifa::FontRef) -> (Vec<Axis>, Vec<NamedInstance>) {
    let all = f.axes();
    let axes: Vec<Axis> = all
        .iter()
        .filter_map(|a| {
            let tag = String::from_utf8(a.tag().to_be_bytes().to_vec()).ok().filter(|t| figcraft_doc::valid_axis_tag(t))?;
            let (min, default, max) = (f64::from(a.min_value()), f64::from(a.default_value()), f64::from(a.max_value()));
            if !(min.is_finite() && default.is_finite() && max.is_finite()) || min > max {
                return None;
            }
            let name = name_string(f, a.name_id()).unwrap_or_else(|| axis_label(&tag).to_string());
            Some(Axis { tag, name, min, default: default.clamp(min, max), max, hidden: a.is_hidden() })
        })
        .collect();
    // A dropped axis would misalign instance coordinates: expose no variations then.
    if axes.len() != all.len() {
        return (Vec::new(), Vec::new());
    }
    let mut instances: Vec<NamedInstance> = Vec::new();
    for inst in f.named_instances().iter().take(512) {
        let Some(name) = name_string(f, inst.subfamily_name_id()) else { continue };
        let coords: Vec<f64> = inst.user_coords().map(f64::from).collect();
        if coords.len() != axes.len() || coords.iter().any(|c| !c.is_finite()) || instances.iter().any(|i| norm_style(&i.name) == norm_style(&name)) {
            continue;
        }
        instances.push(NamedInstance { name, coords });
    }
    (axes, instances)
}

struct FlipPen(BezPath);

impl OutlinePen for FlipPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), -f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), -f64::from(y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to((f64::from(cx0), -f64::from(cy0)), (f64::from(x), -f64::from(y)));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to((f64::from(cx0), -f64::from(cy0)), (f64::from(cx1), -f64::from(cy1)), (f64::from(x), -f64::from(y)));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

fn name_string(f: &skrifa::FontRef, id: StringId) -> Option<String> {
    let s: String = f.localized_strings(id).english_or_first()?.chars().collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// (family, style) preferring the typographic names.
fn names(f: &skrifa::FontRef) -> (Option<String>, Option<String>) {
    let family = name_string(f, StringId::TYPOGRAPHIC_FAMILY_NAME).or_else(|| name_string(f, StringId::FAMILY_NAME));
    let style = name_string(f, StringId::TYPOGRAPHIC_SUBFAMILY_NAME).or_else(|| name_string(f, StringId::SUBFAMILY_NAME));
    (family, style)
}

/// Families tried, in order, for characters a text's own font lacks.
const FALLBACK_FAMILIES: [&str; 18] = [
    "Inter",
    "Helvetica Neue",
    "Arial",
    "Segoe UI",
    "DejaVu Sans",
    "Noto Sans",
    "BIZ UDPGothic",
    "Hiragino Sans",
    "PingFang SC",
    "Noto Sans CJK SC",
    "Microsoft YaHei",
    "Apple SD Gothic Neo",
    "Malgun Gothic",
    "Noto Sans CJK KR",
    "Noto Sans KR",
    "Noto Sans Arabic",
    "Apple Symbols",
    "Segoe UI Symbol",
];

/// A system font file entry not yet loaded.
#[derive(Clone, Debug)]
struct Pending {
    path: std::path::PathBuf,
    index: u32,
    style: String,
}

#[derive(Default)]
pub struct FontDb {
    faces: Vec<Arc<Face>>,
    /// Lower-case family → (display family, face indices).
    families: BTreeMap<String, (String, Vec<usize>)>,
    /// System faces known by name but not loaded (lower-case family → entries).
    pending: BTreeMap<String, (String, Vec<Pending>)>,
    system_scanned: bool,
}

static DB: OnceLock<RwLock<FontDb>> = OnceLock::new();
/// Set by [`scan_system_fonts_on_demand`].
static ON_DEMAND: AtomicBool = AtomicBool::new(false);
/// Done once per process: concurrent callers wait for the one scan.
static SCANNED: OnceLock<()> = OnceLock::new();
/// Directory walks done (at most one).
#[cfg(not(target_arch = "wasm32"))]
static SCANS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The process-wide font database (embedded fonts loaded on first use; system fonts indexed first
/// when [`scan_system_fonts_on_demand`] was called).
pub fn db() -> &'static RwLock<FontDb> {
    if ON_DEMAND.load(Ordering::Relaxed) {
        scan_system_fonts();
    }
    raw_db()
}

fn raw_db() -> &'static RwLock<FontDb> {
    DB.get_or_init(|| {
        let mut db = FontDb::default();
        for f in CRAFT_FONTS {
            if let Some(face) = Face::from_bytes(FontBytes::Static(f.bytes), 0, Some(f.family), Some(f.style)) {
                db.add(face);
            }
        }
        RwLock::new(db)
    })
}

impl FontDb {
    pub fn add(&mut self, face: Face) -> Arc<Face> {
        let key = face.family.to_lowercase();
        let display = face.family.clone();
        let face = Arc::new(face);
        self.faces.push(face.clone());
        let idx = self.faces.len() - 1;
        self.families.entry(key).or_insert_with(|| (display, Vec::new())).1.push(idx);
        face
    }

    /// Add font file bytes (all faces of a collection). Returns the faces added.
    pub fn add_bytes(&mut self, bytes: Vec<u8>) -> Vec<Arc<Face>> {
        let shared = Arc::new(bytes);
        let count = match FileRef::new(shared.as_slice()) {
            Ok(FileRef::Collection(c)) => c.len(),
            Ok(FileRef::Font(_)) => 1,
            Err(_) => 0,
        };
        (0..count).filter_map(|i| Face::from_bytes(FontBytes::Shared(shared.clone()), i, None, None)).map(|f| self.add(f)).collect()
    }

    /// Family display names, sorted (loaded and known system families).
    pub fn families(&self) -> Vec<String> {
        let mut v: Vec<String> = self.families.values().map(|(d, _)| d.clone()).collect();
        for (k, (d, _)) in &self.pending {
            if !self.families.contains_key(k) {
                v.push(d.clone());
            }
        }
        v.sort_by_key(|a| a.to_lowercase());
        v.dedup();
        v
    }

    /// Style names of a family, by weight then italic.
    pub fn styles(&mut self, family: &str) -> Vec<String> {
        self.load_family(family, None);
        let Some((_, idx)) = self.families.get(&family.to_lowercase()) else { return vec![] };
        // (weight, italic, order in the font, name): named instances keep the font's order.
        let mut v: Vec<(u16, bool, usize, String)> = Vec::new();
        for f in idx.iter().filter_map(|i| self.faces.get(*i)) {
            if f.instances.is_empty() {
                v.push((f.weight, f.italic, 0, f.style.clone()));
                continue;
            }
            // A variable font offers its named instances as styles.
            let wght = f.axes.iter().position(|a| a.tag == "wght");
            for (k, inst) in f.instances.iter().enumerate() {
                let w = wght.and_then(|i| inst.coords.get(i)).map(|w| w.round().clamp(1.0, 1000.0) as u16).unwrap_or(f.weight);
                v.push((w, f.italic || inst.name.to_lowercase().contains("italic"), k, inst.name.clone()));
            }
        }
        v.sort();
        let mut seen = std::collections::HashSet::new();
        v.retain(|x| seen.insert(norm_style(&x.3)));
        v.into_iter().map(|x| x.3).collect()
    }

    pub fn has_family(&self, family: &str) -> bool {
        let k = family.to_lowercase();
        self.families.contains_key(&k) || self.pending.contains_key(&k)
    }

    /// Read a system family's faces from disk; with `only`, just the face nearest that style (the
    /// rest stay pending).
    fn load_family(&mut self, family: &str, only: Option<&str>) {
        let key = family.to_lowercase();
        let entries = match only {
            None => match self.pending.remove(&key) {
                Some((_, entries)) => entries,
                None => return,
            },
            Some(style) => {
                let Some((_, pending)) = self.pending.get_mut(&key) else { return };
                let (want, want_w) = (norm_style(style), pending_weight(style));
                let cost = |e: &Pending| {
                    let italic = if e.style.to_lowercase().contains("italic") { 1000 } else { 0 };
                    if norm_style(&e.style) == want { 0 } else { 1 + (pending_weight(&e.style) - want_w).abs() + italic }
                };
                let Some(i) = pending.iter().enumerate().min_by_key(|(_, e)| cost(e)).map(|(i, _)| i) else { return };
                let one = pending.remove(i);
                if pending.is_empty() {
                    self.pending.remove(&key);
                }
                vec![one]
            }
        };
        let mut by_path: BTreeMap<std::path::PathBuf, Arc<Vec<u8>>> = BTreeMap::new();
        for e in entries {
            let bytes = match by_path.get(&e.path) {
                Some(b) => b.clone(),
                None => match std::fs::read(&e.path) {
                    Ok(b) => {
                        let b = Arc::new(b);
                        by_path.insert(e.path.clone(), b.clone());
                        b
                    }
                    Err(_) => continue,
                },
            };
            if let Some(face) = Face::from_bytes(FontBytes::Shared(bytes), e.index, None, Some(&e.style)) {
                self.add(face);
            }
        }
    }

    /// The best face for `family` + `style` (exact style name, else nearest weight/italic).
    pub fn find(&mut self, family: &str, style: &str) -> Option<Arc<Face>> {
        self.load_family(family, None);
        self.best(family, style)
    }

    /// [`find`](Self::find), but a system family not loaded yet reads only its face nearest
    /// `style`: a fallback for missing characters needs one face, not every file of a CJK family
    /// (10–20 MB each). The family's other faces load when it is used by name.
    pub fn find_one(&mut self, family: &str, style: &str) -> Option<Arc<Face>> {
        if !self.families.contains_key(&family.to_lowercase()) {
            self.load_family(family, Some(style));
        }
        self.best(family, style)
    }

    /// The loaded face of `family` nearest `style`.
    fn best(&self, family: &str, style: &str) -> Option<Arc<Face>> {
        let (_, idx) = self.families.get(&family.to_lowercase())?;
        let faces: Vec<&Arc<Face>> = idx.iter().filter_map(|i| self.faces.get(*i)).collect();
        let want = norm_style(style);
        if let Some(f) = faces.iter().find(|f| norm_style(&f.style) == want).or_else(|| faces.iter().find(|f| f.instance(style).is_some())) {
            return Some((*f).clone());
        }
        let want_w = i32::from(figcraft_doc::style_weight(style));
        let want_i = style.to_lowercase().contains("italic");
        // A variable face covers its whole weight range, and an italic or slant axis can slant it.
        let cost = |f: &Arc<Face>| {
            let w = f.axis("wght").map(|a| a.clamp(f64::from(want_w)).round() as i32).unwrap_or(i32::from(f.weight));
            let slants = f.axis("ital").is_some() || f.axis("slnt").is_some_and(|a| a.min < 0.0);
            let style_cost = if f.italic == want_i {
                0
            } else if want_i && slants {
                500
            } else {
                1000
            };
            (w - want_w).abs() + style_cost
        };
        faces.into_iter().min_by_key(|f| cost(f)).cloned()
    }

    /// The default face (Inter Regular if embedded, else any system sans).
    pub fn default_face(&mut self) -> Option<Arc<Face>> {
        for fam in ["Inter", "Helvetica Neue", "Helvetica", "Arial", "Segoe UI", "Roboto", "DejaVu Sans", "Liberation Sans", "Noto Sans"] {
            if let Some(f) = self.find(fam, "Regular") {
                return Some(f);
            }
        }
        self.faces.first().cloned()
    }

    /// Faces to try for characters the primary face lacks.
    pub fn fallbacks(&mut self) -> Vec<Arc<Face>> {
        self.fallbacks_for("Regular")
    }

    /// [`fallbacks`](Self::fallbacks), each family's face nearest `style`, so the characters a
    /// bold run takes from a fallback font (Hangul in Inter Bold) come out bold too.
    pub fn fallbacks_for(&mut self, style: &str) -> Vec<Arc<Face>> {
        let mut out = Vec::new();
        for fam in FALLBACK_FAMILIES {
            if let Some(f) = self.find(fam, style) {
                out.push(f);
            }
        }
        out
    }

    /// Colour emoji faces, for characters no other fallback covers. Kept out of
    /// [`fallbacks`](Self::fallbacks) because the files are large (Apple Color Emoji is ~180 MB)
    /// and only texts that actually show emoji need them; each family reads one face.
    pub fn emoji_fallbacks(&mut self) -> Vec<Arc<Face>> {
        let mut out = Vec::new();
        for fam in ["Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji"] {
            if let Some(f) = self.find_one(fam, "Regular") {
                out.push(f);
            }
        }
        out
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Index the system font directories (desktop only): family names are read now, faces loaded
    /// when first used. Safe to call more than once.
    pub fn scan_system(&mut self) {
        if self.system_scanned {
            return;
        }
        self.system_scanned = true;
        for dir in system_font_dirs() {
            scan_dir(&dir, 0, &mut self.pending);
        }
        // Families already embedded win.
        let embedded: Vec<String> = self.families.keys().cloned().collect();
        for k in embedded {
            self.pending.remove(&k);
        }
    }
}

fn system_font_dirs() -> Vec<std::path::PathBuf> {
    #[allow(unused_mut)]
    let mut v: Vec<std::path::PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        v.push("/System/Library/Fonts".into());
        v.push("/Library/Fonts".into());
        if let Some(h) = std::env::var_os("HOME") {
            v.push(std::path::PathBuf::from(h).join("Library/Fonts"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        let windir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
        v.push(windir.join("Fonts"));
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            v.push(std::path::PathBuf::from(l).join("Microsoft\\Windows\\Fonts"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        v.push("/usr/share/fonts".into());
        v.push("/usr/local/share/fonts".into());
        v.extend(unix_user_font_dirs(std::env::var_os("HOME"), std::env::var_os("XDG_DATA_HOME")));
    }
    v
}

/// The user's own font folder, one [`scan_system_fonts`] reads: where downloaded fonts go.
#[cfg(target_os = "macos")]
pub fn user_font_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/Fonts"))
}

/// The user's own font folder, one [`scan_system_fonts`] reads: where downloaded fonts go.
#[cfg(target_os = "windows")]
pub fn user_font_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|l| std::path::PathBuf::from(l).join("Microsoft\\Windows\\Fonts"))
}

/// The user's own font folder, one [`scan_system_fonts`] reads (`$XDG_DATA_HOME/fonts`): where
/// downloaded fonts go.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn user_font_dir() -> Option<std::path::PathBuf> {
    unix_user_font_dirs(std::env::var_os("HOME"), std::env::var_os("XDG_DATA_HOME")).pop()
}

/// No font folder of the user's here (the browser).
#[cfg(not(any(unix, windows)))]
pub fn user_font_dir() -> Option<std::path::PathBuf> {
    None
}

// Fontconfig's default configuration uses $XDG_DATA_HOME/fonts for user fonts.
// Ignore relative XDG paths, as required by the XDG Base Directory Specification.
#[cfg(all(unix, not(target_os = "macos")))]
fn unix_user_font_dirs(home: Option<std::ffi::OsString>, data_home: Option<std::ffi::OsString>) -> Vec<std::path::PathBuf> {
    let home = home.map(std::path::PathBuf::from);
    let data = data_home.map(std::path::PathBuf::from).filter(|p| p.is_absolute()).or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    let mut dirs = Vec::new();
    if let Some(home) = home {
        dirs.push(home.join(".fonts"));
    }
    if let Some(data) = data {
        dirs.push(data.join("fonts"));
    }
    dirs
}

fn scan_dir(dir: &std::path::Path, depth: usize, out: &mut BTreeMap<String, (String, Vec<Pending>)>) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        // The listing's own metadata (free on Windows); a symlink's target needs a lookup.
        let meta = match entry.file_type() {
            Ok(t) if !t.is_symlink() => entry.metadata(),
            _ => std::fs::metadata(&path),
        };
        let Ok(meta) = meta else { continue };
        if meta.is_dir() {
            scan_dir(&path, depth + 1, out);
            continue;
        }
        // Font-like names can also belong to pipes or devices; opening a pipe can block.
        // `meta` follows symlinks, so links to regular font files remain supported.
        if !meta.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
            continue;
        }
        // Very large files are indexed all the same (Apple Color Emoji is ~180 MB and is where the
        // system's colour emoji live): `index_bytes` reads headers and name tables out of a zeroed
        // buffer, so only those parts are ever touched.
        let Some(bytes) = usize::try_from(meta.len()).ok().filter(|l| *l <= 256 * 1024 * 1024).and_then(|l| index_bytes(&path, l)) else { continue };
        for (index, family, style) in faces_in(&bytes) {
            out.entry(family.to_lowercase()).or_insert_with(|| (family.clone(), Vec::new())).1.push(Pending { path: path.clone(), index, style });
        }
    }
}

/// The parts of a font file the scan reads: headers, table directories and `name` tables, at their
/// offsets in a zeroed buffer of the file's length (reading whole files was most of the scan).
fn index_bytes(path: &std::path::Path, len: usize) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; len];
    let mut fill = |buf: &mut Vec<u8>, at: usize, n: usize| -> Option<()> {
        let dst = buf.get_mut(at..at.checked_add(n)?)?;
        file.seek(SeekFrom::Start(u64::try_from(at).ok()?)).ok()?;
        file.read_exact(dst).ok()
    };
    let be = |b: &[u8], at: usize, n: usize| b.get(at..at.checked_add(n)?).map(|s| s.iter().fold(0usize, |v, x| v << 8 | usize::from(*x)));
    fill(&mut buf, 0, 12)?;
    let fonts: Vec<usize> = if buf.get(..4) == Some(b"ttcf") {
        let n = be(&buf, 8, 4)?.min(64);
        fill(&mut buf, 12, 4 * n)?;
        (0..n).filter_map(|i| be(&buf, 12 + 4 * i, 4)).collect()
    } else {
        vec![0]
    };
    for at in fonts {
        let Some(tables) = fill(&mut buf, at, 12).and_then(|()| be(&buf, at + 4, 2)) else { continue };
        if fill(&mut buf, at + 12, 16 * tables).is_none() {
            continue;
        }
        for rec in (0..tables).map(|t| at + 12 + 16 * t) {
            if buf.get(rec..rec + 4) == Some(b"name")
                && let (Some(off), Some(n)) = (be(&buf, rec + 8, 4), be(&buf, rec + 12, 4))
            {
                let _ = fill(&mut buf, off, n);
            }
        }
    }
    Some(buf)
}

/// (index, family, style) of each face in a font file: outline fonts and colour fonts (Apple Color
/// Emoji draws most emoji from `sbix`, Noto Color Emoji has no outlines at all).
fn faces_in(bytes: &[u8]) -> Vec<(u32, String, String)> {
    let count = match FileRef::new(bytes) {
        Ok(FileRef::Collection(c)) => c.len(),
        Ok(FileRef::Font(_)) => 1,
        Err(_) => 0,
    };
    (0..count.min(64))
        .filter_map(|i| {
            let f = skrifa::FontRef::from_index(bytes, i).ok()?;
            [b"glyf", b"CFF ", b"CFF2", b"sbix", b"CBDT", b"EBDT", b"COLR"]
                .iter()
                .any(|t| f.data_for_tag(skrifa::Tag::new(t)).is_some())
                .then_some(())?;
            let (family, style) = names(&f);
            let family = family.filter(|f| !f.starts_with('.'))?;
            Some((i, family, style.unwrap_or_else(|| "Regular".into())))
        })
        .collect()
}

/// Convenience: find a face with the global db.
pub fn find_face(family: &str, style: &str) -> Option<Arc<Face>> {
    let mut db = db().write().unwrap_or_else(std::sync::PoisonError::into_inner);
    db.find(family, style).or_else(|| db.default_face())
}

/// All family names known to the global db.
pub fn family_names() -> Vec<String> {
    db().read().unwrap_or_else(std::sync::PoisonError::into_inner).families()
}

/// The global db has `family` (embedded, loaded or indexed; any case).
pub fn has_family(family: &str) -> bool {
    db().read().unwrap_or_else(std::sync::PoisonError::into_inner).has_family(family)
}

/// Style names of a family in the global db.
pub fn family_styles(family: &str) -> Vec<String> {
    db().write().unwrap_or_else(std::sync::PoisonError::into_inner).styles(family)
}

/// Scan system fonts into the global db (desktop; a no-op on the web, or with
/// `FIGCRAFT_NO_SYSTEM_FONTS` set). Runs once per process: a concurrent caller waits for that scan.
/// The directory walk runs without holding the database lock; results are merged in one short write.
pub fn scan_system_fonts() {
    SCANNED.get_or_init(|| {
        #[cfg(not(target_arch = "wasm32"))]
        if !system_fonts_opted_out() {
            SCANS.fetch_add(1, Ordering::Relaxed);
            let mut found: BTreeMap<String, (String, Vec<Pending>)> = BTreeMap::new();
            for dir in system_font_dirs() {
                scan_dir(&dir, 0, &mut found);
            }
            let mut w = raw_db().write().unwrap_or_else(std::sync::PoisonError::into_inner);
            w.system_scanned = true;
            for (k, v) in found {
                if !w.families.contains_key(&k) {
                    w.pending.entry(k).or_insert(v);
                }
            }
            drop(w);
            crate::clear_layout_cache();
        }
    });
}

/// `FIGCRAFT_NO_SYSTEM_FONTS` is set (and not `0`): renders use only the embedded fonts.
#[cfg(not(target_arch = "wasm32"))]
fn system_fonts_opted_out() -> bool {
    std::env::var_os("FIGCRAFT_NO_SYSTEM_FONTS").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Every font text can use is known: the system fonts were indexed, or there are none to index
/// (opted out, or the web). Until then a family the scan would find counts as missing.
pub fn installed_fonts_known() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    if !system_fonts_opted_out() {
        return system_fonts_scanned();
    }
    true
}

/// Index the system fonts the first time the font database is used (headless apps: a run that
/// lays out no text never pays for the scan). The desktop app scans in the background instead.
pub fn scan_system_fonts_on_demand() {
    ON_DEMAND.store(true, Ordering::Relaxed);
}

/// Whether the system fonts were indexed ([`scan_system_fonts`] finished; always false on the web).
pub fn system_fonts_scanned() -> bool {
    raw_db().read().unwrap_or_else(std::sync::PoisonError::into_inner).system_scanned
}

/// The first of `families` (embedded or system) whose regular face draws `c`, for a UI toolkit
/// that needs a font for a script its own fonts lack.
pub fn face_with_char(c: char, families: &[&str]) -> Option<Arc<Face>> {
    let mut db = db().write().unwrap_or_else(std::sync::PoisonError::into_inner);
    families.iter().filter_map(|f| db.find_one(f, "Regular")).find(|f| f.has_char(c))
}

#[cfg(test)]
#[path = "fontdb_tests.rs"]
mod tests;
