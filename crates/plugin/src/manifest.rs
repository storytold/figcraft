//! The plugin manifest, `figcraft-plugin.json`: who the plugin is, which module to run, the menu
//! commands it adds (with their parameters) and the permissions it asks for. Also the panel a
//! plugin shows while it runs (ABI v2), whose controls are parameters too.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The manifest's file name.
pub const MANIFEST_FILE: &str = "figcraft-plugin.json";
/// The plugin API version this build implements (`api` in the manifest).
pub const API_VERSION: u32 = 3;
/// Largest manifest read.
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

const MAX_ID: usize = 128;
const MAX_NAME: usize = 64;
const MAX_LABEL: usize = 80;
const MAX_TEXT: usize = 1000;
const MAX_COMMANDS: usize = 64;
const MAX_PARAMS: usize = 32;
const MAX_OPTIONS: usize = 64;
const MAX_ALLOW: usize = 256;
/// Largest panel spec a plugin may show, and its most controls.
pub const MAX_PANEL_BYTES: usize = 64 * 1024;
const MAX_CONTROLS: usize = 64;
/// Longest text parameter value.
const MAX_TEXT_VALUE: usize = 10_000;

/// A parsed, validated manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Stable id, e.g. `org.example.tidy-names` (letters, digits, `.`, `-`, `_`).
    pub id: String,
    /// Shown in the Plugins menu.
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Who made it, as they typed it (plugin directories show it; nothing checks it).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// The WebAssembly module, relative to the manifest (`plugin.wasm`).
    pub main: String,
    /// The plugin API version the plugin was written for (1 when not given; panels and settings
    /// need 2, widgets 3).
    #[serde(default = "api_version", deserialize_with = "api_from_number_or_string")]
    pub api: u32,
    #[serde(default)]
    pub permissions: Permissions,
    /// Menu commands; a manifest without any gets one, `run` (a widget needs none).
    #[serde(default)]
    pub commands: Vec<PluginCommand>,
    /// API 3: the plugin draws a widget (Plugins ▸ Widgets, `widget.insert`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub widget: bool,
}

/// A manifest without `api` was written for the first one.
fn api_version() -> u32 {
    1
}

/// `api` as a whole number or a version string (`"1.0.0"`, whose major part is the API version).
fn api_from_number_or_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Api {
        Number(u32),
        Text(String),
    }
    match Api::deserialize(d)? {
        Api::Number(n) => Ok(n),
        Api::Text(s) => s
            .trim()
            .split('.')
            .next()
            .and_then(|major| major.parse().ok())
            .ok_or_else(|| serde::de::Error::custom(format!("invalid api version `{s}`"))),
    }
}

/// What a plugin may do.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Permissions {
    /// `read`: commands that leave the document as it is; `write`: edits too.
    #[serde(default)]
    pub access: Access,
    /// The commands the plugin may call: exact ids or prefixes ending in `*` (`node.*`). Empty: any
    /// command its access allows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
}

impl Permissions {
    /// Whether the allowlist admits `command`.
    pub fn allows(&self, command: &str) -> bool {
        self.commands.is_empty()
            || self.commands.iter().any(|p| match p.strip_suffix('*') {
                Some(prefix) => command.starts_with(prefix),
                None => p == command,
            })
    }
}

/// Read-only or read-write document access.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    #[default]
    Read,
    Write,
}

/// One menu command of a plugin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginCommand {
    /// Passed to the plugin as `command`.
    pub id: String,
    pub label: String,
    /// Parameters asked for before the command runs (a small form in the app).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<Param>,
}

/// A command parameter: a form field in the app, a key of `params` everywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Param {
    /// Empty only for a panel's notes.
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(rename = "type", default)]
    pub kind: ParamKind,
    /// In a panel, the control's current value (`value` there).
    #[serde(default, alias = "value", skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// The choices of a `select` or `list`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Bounds of a `number`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

/// The kind of value a parameter takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamKind {
    #[default]
    Text,
    Number,
    Boolean,
    Select,
    /// A hex colour, `#RRGGBB` or `#RRGGBBAA`.
    Color,
    /// One of `options`, shown as a list rather than a menu.
    List,
    /// Panels only: a button (it has no value).
    Button,
    /// Panels only: text (`label`, up to 1000 characters).
    Note,
}

impl Param {
    /// The label shown in the form.
    pub fn title(&self) -> &str {
        if self.label.is_empty() { &self.name } else { &self.label }
    }

    /// Whether it holds a value (not a button or a note).
    pub fn is_field(&self) -> bool {
        !matches!(self.kind, ParamKind::Button | ParamKind::Note)
    }

    /// The value used when none is given.
    pub fn default_value(&self) -> Value {
        if let Some(d) = self.default.as_ref().and_then(|d| self.coerce(d).ok()) {
            return d;
        }
        match self.kind {
            ParamKind::Text => Value::String(String::new()),
            ParamKind::Number => serde_json::json!(self.clamp(0.0)),
            ParamKind::Boolean => Value::Bool(false),
            ParamKind::Select | ParamKind::List => self.options.first().map_or(Value::Null, |o| Value::String(o.clone())),
            ParamKind::Color => Value::String("#000000".into()),
            ParamKind::Button | ParamKind::Note => Value::Null,
        }
    }

    /// `v` as this parameter's kind, or why it can't be.
    pub fn coerce(&self, v: &Value) -> Result<Value, String> {
        let name = &self.name;
        match self.kind {
            ParamKind::Text => match v {
                Value::String(s) if s.chars().count() <= MAX_TEXT_VALUE => Ok(v.clone()),
                Value::String(_) => Err(format!("`{name}` is longer than {MAX_TEXT_VALUE} characters")),
                Value::Number(n) => Ok(Value::String(n.to_string())),
                _ => Err(format!("`{name}` must be text")),
            },
            ParamKind::Number => {
                let n = match v {
                    Value::Number(n) => n.as_f64(),
                    Value::String(s) => s.trim().parse::<f64>().ok(),
                    _ => None,
                }
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("`{name}` must be a number"))?;
                Ok(serde_json::json!(self.clamp(n)))
            }
            ParamKind::Boolean => v.as_bool().map(Value::Bool).ok_or_else(|| format!("`{name}` must be true or false")),
            ParamKind::Select | ParamKind::List => match v.as_str() {
                Some(s) if self.options.iter().any(|o| o == s) => Ok(v.clone()),
                _ => Err(format!("`{name}` must be one of: {}", self.options.join(", "))),
            },
            ParamKind::Color => v
                .as_str()
                .filter(|s| s.len() <= 9)
                .and_then(figcraft_color::Color::from_hex)
                .map(|c| Value::String(c.to_css_hex()))
                .ok_or_else(|| format!("`{name}` must be a hex colour like #FF5A5F")),
            ParamKind::Button | ParamKind::Note => Ok(Value::Null),
        }
    }

    /// Check a parameter or panel control: its name, label, options and bounds. A default that
    /// doesn't fit its kind is dropped.
    fn check(&mut self) -> Result<(), String> {
        if !(self.kind == ParamKind::Note && self.name.is_empty()) {
            check_id("parameter name", &self.name, MAX_NAME)?;
        }
        let max = if self.kind == ParamKind::Note { MAX_TEXT } else { MAX_LABEL };
        self.label = clean("parameter label", &self.label, max)?;
        if self.options.len() > MAX_OPTIONS {
            return Err(format!("parameter `{}` has more than {MAX_OPTIONS} options", self.name));
        }
        for o in &mut self.options {
            *o = clean("option", o, MAX_LABEL)?;
        }
        if matches!(self.kind, ParamKind::Select | ParamKind::List) && self.options.is_empty() {
            return Err(format!("parameter `{}` has no options", self.name));
        }
        if self.min.is_some_and(|v| !v.is_finite())
            || self.max.is_some_and(|v| !v.is_finite())
            || matches!((self.min, self.max), (Some(a), Some(b)) if a > b)
        {
            return Err(format!("parameter `{}` has bad bounds", self.name));
        }
        self.default = self.default.as_ref().and_then(|d| self.coerce(d).ok());
        Ok(())
    }

    /// `n` within the number's bounds.
    fn clamp(&self, n: f64) -> f64 {
        n.max(self.min.unwrap_or(f64::MIN)).min(self.max.unwrap_or(f64::MAX))
    }
}

impl PluginCommand {
    /// `given` with every declared parameter checked and the missing ones filled with their
    /// defaults. Keys the command doesn't declare pass through.
    pub fn resolve_params(&self, given: &Value) -> Result<Value, String> {
        let mut out = match given {
            Value::Object(m) => m.clone(),
            Value::Null => Map::new(),
            _ => return Err("`params` must be an object".into()),
        };
        for p in &self.params {
            let v = match out.get(&p.name) {
                Some(v) if !v.is_null() => p.coerce(v)?,
                _ => p.default_value(),
            };
            out.insert(p.name.clone(), v);
        }
        Ok(Value::Object(out))
    }
}

impl Manifest {
    /// Parse and validate manifest JSON.
    pub fn parse(text: &str) -> Result<Manifest, String> {
        if text.len() > MAX_MANIFEST_BYTES {
            return Err(format!("the manifest is larger than {} KB", MAX_MANIFEST_BYTES / 1024));
        }
        let m: Manifest = serde_json::from_str(text).map_err(|e| format!("manifest: {e}"))?;
        m.validated()
    }

    /// Validate a manifest given as JSON.
    pub fn from_value(v: &Value) -> Result<Manifest, String> {
        Manifest::parse(&v.to_string())
    }

    /// The command `id`, or the first one when `id` is `None` or empty.
    pub fn command(&self, id: Option<&str>) -> Option<&PluginCommand> {
        match id.filter(|i| !i.is_empty()) {
            Some(id) => self.commands.iter().find(|c| c.id == id),
            None => self.commands.first(),
        }
    }

    /// Check every field and fill in the defaults (a `run` command when there is none).
    fn validated(mut self) -> Result<Manifest, String> {
        self.check().map_err(|e| format!("manifest: {e}"))?;
        Ok(self)
    }

    fn check(&mut self) -> Result<(), String> {
        check_id("id", &self.id, MAX_ID)?;
        self.name = clean("name", &self.name, MAX_NAME)?;
        if self.name.is_empty() {
            return Err("`name` is empty".into());
        }
        self.version = clean("version", &self.version, 32)?;
        self.description = clean("description", &self.description, MAX_TEXT)?;
        self.author = clean("author", &self.author, MAX_NAME)?;
        check_main(&self.main)?;
        if self.api == 0 || self.api > API_VERSION {
            return Err(format!("plugin API {} isn't supported (this FigCraft runs API {API_VERSION})", self.api));
        }
        if self.permissions.commands.len() > MAX_ALLOW {
            return Err(format!("more than {MAX_ALLOW} entries in `permissions.commands`"));
        }
        for p in &self.permissions.commands {
            let bare = p.strip_suffix('*').unwrap_or(p);
            if p.is_empty() || p.len() > MAX_ID || bare.contains('*') || !bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
                return Err(format!("`{}` in `permissions.commands` isn't a command id or `prefix.*`", truncate(p)));
            }
        }
        if self.widget && self.api < 3 {
            return Err("a widget needs plugin API 3 (`\"api\": 3`)".into());
        }
        if self.commands.is_empty() && !self.widget {
            self.commands.push(PluginCommand { id: "run".into(), label: "Run".into(), params: Vec::new() });
        }
        if self.commands.len() > MAX_COMMANDS {
            return Err(format!("more than {MAX_COMMANDS} commands"));
        }
        if let Some(dup) = first_duplicate(self.commands.iter().map(|c| c.id.as_str())) {
            return Err(format!("two commands are called `{}`", truncate(dup)));
        }
        for c in &mut self.commands {
            check_id("command id", &c.id, MAX_NAME)?;
            c.label = clean("command label", &c.label, MAX_LABEL)?;
            if c.label.is_empty() {
                c.label = c.id.clone();
            }
            if c.params.len() > MAX_PARAMS {
                return Err(format!("command `{}` has more than {MAX_PARAMS} parameters", c.id));
            }
            for p in &mut c.params {
                p.check()?;
                if !p.is_field() {
                    return Err(format!("parameter `{}`: buttons and notes only go in panels", p.name));
                }
            }
            if let Some(dup) = first_duplicate(c.params.iter().map(|p| p.name.as_str())) {
                return Err(format!("command `{}` declares `{dup}` twice", c.id));
            }
        }
        Ok(())
    }
}

/// A panel a plugin shows while it runs (ABI v2): a title and controls drawn with FigCraft's own
/// widgets. Fields hold values; a button press or a changed value calls the plugin back.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default)]
    pub controls: Vec<Param>,
}

impl Panel {
    /// Parse and validate a panel spec (it comes from the plugin: untrusted).
    pub fn parse(text: &str) -> Result<Panel, String> {
        if text.len() > MAX_PANEL_BYTES {
            return Err(format!("the panel is larger than {} KB", MAX_PANEL_BYTES / 1024));
        }
        let mut p: Panel = serde_json::from_str(text).map_err(|e| format!("panel: {e}"))?;
        p.title = clean("title", &p.title, MAX_NAME).map_err(|e| format!("panel: {e}"))?;
        if p.controls.len() > MAX_CONTROLS {
            return Err(format!("panel: more than {MAX_CONTROLS} controls"));
        }
        for c in &mut p.controls {
            c.check().map_err(|e| format!("panel: {e}"))?;
        }
        if let Some(dup) = first_duplicate(p.controls.iter().map(|c| c.name.as_str()).filter(|n| !n.is_empty())) {
            return Err(format!("panel: two controls are called `{dup}`"));
        }
        Ok(p)
    }

    /// The control called `name`.
    pub fn control(&self, name: &str) -> Option<&Param> {
        self.controls.iter().find(|c| !c.name.is_empty() && c.name == name)
    }

    /// Its fields' values: a field's own value when the spec gives one, else its value in `old`
    /// (what the user entered before the plugin updated the panel), else its default.
    pub fn values(&self, old: &Map<String, Value>) -> Map<String, Value> {
        self.controls
            .iter()
            .filter(|c| c.is_field())
            .map(|c| {
                let v = c.default.clone().or_else(|| old.get(&c.name).and_then(|v| c.coerce(v).ok())).unwrap_or_else(|| c.default_value());
                (c.name.clone(), v)
            })
            .collect()
    }
}

/// The first name that occurs twice.
fn first_duplicate<'a>(names: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut seen = std::collections::BTreeSet::new();
    names.into_iter().find(|n| !seen.insert(*n))
}

/// An id: 1..=`max` ASCII letters, digits, `.`, `-` or `_`.
pub(crate) fn check_id(what: &str, id: &str, max: usize) -> Result<(), String> {
    if id.is_empty() || id.len() > max || !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) {
        return Err(format!("{what} `{}` must be 1–{max} letters, digits, `.`, `-` or `_`", truncate(id)));
    }
    Ok(())
}

/// Trimmed text of at most `max` characters without control characters.
pub(crate) fn clean(what: &str, s: &str, max: usize) -> Result<String, String> {
    let s = s.trim();
    if s.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    if s.chars().any(char::is_control) {
        return Err(format!("{what} contains control characters"));
    }
    Ok(s.to_string())
}

/// `main` names a `.wasm` file next to (or below) the manifest.
fn check_main(main: &str) -> Result<(), String> {
    let bad = main.is_empty()
        || main.len() > 256
        || !main.to_ascii_lowercase().ends_with(".wasm")
        || main.starts_with(['/', '\\'])
        || main.contains(':')
        || main.chars().any(char::is_control)
        || main.split(['/', '\\']).any(|part| part == ".." || part.is_empty());
    if bad {
        return Err(format!("`main` must be a relative path to a .wasm file inside the plugin's folder (got `{}`)", truncate(main)));
    }
    Ok(())
}

/// At most 40 characters of `s`, for error messages.
fn truncate(s: &str) -> String {
    let mut out: String = s.chars().take(40).collect();
    if s.chars().count() > 40 {
        out.push('…');
    }
    out
}
