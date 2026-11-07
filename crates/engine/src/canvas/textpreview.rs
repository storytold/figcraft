//! Live previews: what a command would do, shown on the canvas without an undo step — while the
//! pointer is over a font family or style, a text style in the style browser, a font's named
//! instance or a blend mode, or while a variable font axis slider is dragged. The document under
//! the preview is kept and comes back when the preview ends ([`end`], `text.endPreview`). Every
//! other command ends it first, so committing (running the command for real) applies to the
//! document itself as one undo step, and Escape just ends the preview.

use std::sync::Arc;

use serde_json::Value;

use crate::{EngineError, Result, Session, bad, find_command, guard, normalize};

/// The commands that show or end a preview (they don't end the one showing).
pub const COMMANDS: [&str; 3] = ["text.preview", "text.endPreview", "widget.hover"];

/// What can be previewed: text styles, font styles and axis values, layers' properties and the
/// blend of a paint on a layer, a selection or a style.
pub const PREVIEWABLE: [&str; 9] = [
    "style.apply",
    "library.applyStyle",
    "text.setVariations",
    "text.style",
    "node.set",
    "fill.set",
    "stroke.set",
    "selection.setPaint",
    "style.update",
];

/// Show `command params` as a live preview (in place of an earlier one).
pub fn show(s: &mut Session, command: &str, params: &Value) -> Result<()> {
    let cmd = "text.preview";
    if !PREVIEWABLE.contains(&command) {
        return Err(bad(cmd, format!("`{command}` can't be previewed (one of: {})", PREVIEWABLE.join(", "))));
    }
    let spec = find_command(command).ok_or_else(|| EngineError::UnknownCommand(command.to_string()))?;
    end(s);
    let ds = s.ds()?;
    if ds.preview.is_some() {
        return Err(EngineError::Disabled(cmd.into(), crate::PREVIEW_READ_ONLY.into()));
    }
    let (base, selection, text_edit, page, uid) = (ds.doc.clone(), ds.selection.clone(), ds.text_edit.clone(), ds.page, ds.uid);
    let result = guard::run(command, || (spec.run)(s, params))
        // Library copies and instance structure stay as they are, in previews too.
        .and_then(|v| s.check_instance_structure(command, &Some((base.clone(), selection.clone(), page, uid))).map(|()| v));
    let ds = s.ds_mut()?;
    // A preview changes what's shown, never the selection or the text being edited.
    ds.selection = selection;
    ds.text_edit = text_edit;
    match result {
        Ok(_) if !Arc::ptr_eq(&ds.doc, &base) => {
            // A panic while normalising shows no preview (#1085).
            if let Err(e) = guard::run(command, || {
                normalize(ds.doc_mut(), &base);
                Ok(())
            }) {
                ds.doc = base;
                return Err(e);
            }
            ds.text_preview = Some(base);
            ds.revision += 1;
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(e) => {
            ds.doc = base;
            Err(e)
        }
    }
}

/// Show what `change` does to the document as a live preview (in place of an earlier one), like
/// [`show`] does with a command's: no undo step, and the next command puts the document back first.
pub(crate) fn show_change(s: &mut Session, cmd: &str, change: impl FnOnce(&mut figcraft_doc::Document) -> Result<()>) -> Result<()> {
    end(s);
    let ds = s.ds_mut()?;
    if ds.preview.is_some() {
        return Err(EngineError::Disabled(cmd.into(), crate::PREVIEW_READ_ONLY.into()));
    }
    let base = ds.doc.clone();
    let mut doc = (*base).clone();
    guard::run(cmd, || {
        change(&mut doc)?;
        normalize(&mut doc, &base);
        Ok(())
    })?;
    ds.doc = Arc::new(doc);
    ds.text_preview = Some(base);
    ds.revision += 1;
    Ok(())
}

/// End the live preview, if any: the document is back as it was. Returns whether one was showing.
pub fn end(s: &mut Session) -> bool {
    let Ok(ds) = s.ds_mut() else { return false };
    let Some(base) = ds.text_preview.take() else { return false };
    ds.doc = base;
    ds.revision += 1;
    true
}

/// Whether a live text preview is showing.
pub fn active(s: &Session) -> bool {
    s.ds().is_ok_and(|d| d.text_preview.is_some())
}
