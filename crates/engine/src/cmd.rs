//! The command registry. Each area module exposes `specs()`; [`all_commands`] collects them once.

pub mod arrange;
pub mod comment;
pub mod component;
pub mod create;
pub mod edit;
pub mod facilitate;
pub mod file;
pub mod image;
pub mod layout;
pub mod library;
pub mod lists;
pub mod page;
pub mod props;
pub mod proto;
pub mod proto_actions;
pub mod section;
pub mod structure;
pub mod styles;
pub mod text;
pub mod variables;
pub mod variables_io;
pub mod view;

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use crate::{Result, Session};

pub type RunFn = fn(&mut Session, &Value) -> Result<Value>;

/// A registered command.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Parameter documentation for agents (`list_commands`).
    pub params: &'static str,
    /// Whether the command edits the document (an undo step).
    pub history: bool,
    pub run: RunFn,
}

impl CommandSpec {
    pub fn menu(&self) -> Option<&'static str> {
        crate::catalog::MENU.iter().find(|m| m.id == self.id).map(|m| m.menu)
    }
    pub fn shortcut(&self) -> Option<&'static str> {
        crate::catalog::shortcut(self.id)
    }
}

/// Define a command spec: `spec!(id, label, params_doc, history, run)`.
#[macro_export]
macro_rules! spec {
    ($id:expr, $label:expr, $params:expr, $hist:expr, $run:expr) => {
        $crate::cmd::CommandSpec { id: $id, label: $label, params: $params, history: $hist, run: $run }
    };
}

struct Registry {
    list: Vec<CommandSpec>,
    index: HashMap<&'static str, usize>,
    /// The parameter names each command's doc lists (`list` order; see [`doc_param_names`]).
    names: Vec<Option<Vec<&'static str>>>,
}

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| {
        let mut list = Vec::new();
        list.extend(create::specs());
        list.extend(edit::specs());
        list.extend(lists::specs());
        list.extend(crate::selection_colors::specs());
        list.extend(arrange::specs());
        list.extend(structure::specs());
        list.extend(section::specs());
        list.extend(layout::specs());
        list.extend(file::specs());
        list.extend(page::specs());
        list.extend(view::specs());
        list.extend(text::specs());
        list.extend(crate::fonts::specs());
        list.extend(styles::specs());
        list.extend(image::specs());
        list.extend(variables::specs());
        list.extend(variables_io::specs());
        list.extend(component::specs());
        list.extend(library::specs());
        list.extend(proto::specs());
        list.extend(proto_actions::specs());
        list.extend(comment::specs());
        list.extend(crate::vectoredit::specs());
        list.extend(crate::shapebuilder::specs());
        list.extend(crate::codegen::specs());
        list.extend(crate::devinspect::specs());
        list.extend(crate::component_codegen::specs());
        list.extend(crate::playground::specs());
        list.extend(crate::code_connect::specs());
        list.extend(crate::annotations::specs());
        list.extend(crate::dev_resources::specs());
        list.extend(crate::dev_measurements::specs());
        list.extend(crate::distance::specs());
        list.extend(crate::collab::specs());
        list.extend(crate::branches::specs());
        list.extend(crate::plugins::specs());
        list.extend(crate::plugin_directory::specs());
        list.extend(crate::widgets::specs());
        list.extend(crate::live::specs());
        list.extend(crate::online::specs());
        list.extend(crate::invite_links::specs());
        list.extend(crate::templates::specs());
        list.extend(crate::datafill::specs());
        list.extend(crate::textfit::specs());
        list.extend(crate::board::specs());
        list.extend(facilitate::specs());
        list.extend(crate::board_table::specs());
        list.extend(crate::board_templates::specs());
        let mut index = HashMap::new();
        for (i, s) in list.iter().enumerate() {
            if index.insert(s.id, i).is_some() {
                log::warn!("duplicate command id {}", s.id);
            }
        }
        let names = list.iter().map(|s| doc_param_names(s.params)).collect();
        Registry { list, index, names }
    })
}

pub fn all_commands() -> &'static [CommandSpec] {
    &registry().list
}

pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    let r = registry();
    r.index.get(id).and_then(|i| r.list.get(*i))
}

/// The parameter names command `id` takes, from its doc; `None` when the doc isn't a `{…}` list
/// (`node.set` documents its props, and refuses the ones it doesn't know itself).
pub fn param_names(id: &str) -> Option<&'static [&'static str]> {
    let r = registry();
    r.index.get(id).and_then(|i| r.names.get(*i)).and_then(|n| n.as_deref())
}

/// Refuse params naming something the command doesn't take: a misspelt name would otherwise be
/// ignored and the command answer as if it hadn't been given. `ids` and `id` are always taken:
/// the layers a command acts on (`params::targets` reads either).
pub fn refuse_unknown_params(id: &str, params: &Value) -> Result<()> {
    let (Some(names), Some(given)) = (param_names(id), params.as_object()) else { return Ok(()) };
    let taken = |k: &str| matches!(k, "ids" | "id") || names.contains(&k);
    let unknown: Vec<String> = given.keys().filter(|k| !taken(k)).map(|k| format!("`{k}`")).collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let s = if unknown.len() == 1 { "" } else { "s" };
    let takes = if names.is_empty() { "none".to_string() } else { names.join(", ") };
    Err(crate::bad(id, format!("unknown parameter{s} {} (it takes {takes})", unknown.join(", "))))
}

/// The top-level items of a `{…}` params doc (`{ids?, direction? (horizontal|vertical)}` →
/// `ids?`, ` direction? (horizontal|vertical)`), with those of the alternatives after it
/// (`{x, y} | {points}`); `None` when the doc doesn't start with one.
pub(crate) fn doc_items(doc: &'static str) -> Option<Vec<&'static str>> {
    let mut body = doc.trim_start().strip_prefix('{')?;
    let (mut items, mut depth, mut start) = (Vec::new(), 0usize, 0);
    loop {
        let mut rest = None;
        for (i, c) in body.char_indices() {
            match c {
                '{' | '[' | '(' => depth += 1,
                '}' | ']' | ')' if depth > 0 => depth -= 1,
                '}' | ',' if depth == 0 => {
                    items.push(body.get(start..i).unwrap_or_default());
                    start = i + c.len_utf8();
                    if c == '}' {
                        rest = body.get(start..);
                        break;
                    }
                }
                _ => {}
            }
        }
        let rest = rest?;
        match rest.trim_start().strip_prefix('|').and_then(|r| r.trim_start().strip_prefix('{')) {
            Some(next) => (body, start) = (next, 0),
            None => return Some(items),
        }
    }
}

/// The names one doc item gives: its first word and the words joined to it by `|`, `/` or `+`
/// (`id?/ids?`, `path | data (base64) + name`, `value? + newValue?`), without the `?` of optional
/// ones. Bracketed text describes a name, and what follows a `:`, `=` or `→` its value.
pub(crate) fn item_names(item: &'static str) -> Vec<&'static str> {
    let mut outside = Vec::new();
    let (mut depth, mut start) = (0usize, None);
    for (i, c) in item.char_indices() {
        let value_follows = depth == 0 && matches!(c, ':' | '=' | '→');
        let in_word = depth == 0 && !value_follows && !c.is_whitespace() && !matches!(c, '(' | '[' | '{' | ')' | ']' | '}');
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        match (in_word, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                outside.push(item.get(s..i).unwrap_or_default());
                start = None;
            }
            _ => {}
        }
        if value_follows {
            break;
        }
    }
    if let Some(s) = start {
        outside.push(item.get(s..).unwrap_or_default());
    }
    let is_name =
        |n: &&str| n.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let mut words = outside.into_iter();
    let (mut names, mut word) = (Vec::new(), words.next());
    while let Some(w) = word {
        names.extend(w.split(['/', '|']).map(|n| n.trim_end_matches('?')).filter(is_name));
        word = match words.next() {
            Some("|" | "/" | "+") => words.next(),
            _ => None,
        };
    }
    names
}

/// Every name a `{…}` params doc lists, once each (see [`item_names`]).
fn doc_param_names(doc: &'static str) -> Option<Vec<&'static str>> {
    let mut names = Vec::new();
    for n in doc_items(doc)?.into_iter().flat_map(item_names) {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    Some(names)
}

/// `{"ok": true}` shorthand.
pub fn ok() -> Result<Value> {
    Ok(serde_json::json!({"ok": true}))
}
