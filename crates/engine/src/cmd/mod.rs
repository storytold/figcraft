//! The command registry. Each area module exposes `specs()`; [`all_commands`] collects them once.

pub mod arrange;
pub mod comment;
pub mod component;
pub mod create;
pub mod edit;
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
        let mut index = HashMap::new();
        for (i, s) in list.iter().enumerate() {
            if index.insert(s.id, i).is_some() {
                log::warn!("duplicate command id {}", s.id);
            }
        }
        Registry { list, index }
    })
}

pub fn all_commands() -> &'static [CommandSpec] {
    &registry().list
}

pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    let r = registry();
    r.index.get(id).and_then(|i| r.list.get(*i))
}

/// `{"ok": true}` shorthand.
pub fn ok() -> Result<Value> {
    Ok(serde_json::json!({"ok": true}))
}
