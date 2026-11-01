//! Text menu and text editing commands.

use figcraft_doc::{
    LetterSpacing, LineHeight, ListStyle, NodeId, NodeKind, TextAlignH, TextAutoResize, TextCase, TextData, TextDecoration, TextPath, TextStyle,
    VariableId,
};
use serde_json::{Map, Value, json};

use super::props::{styled_name, weight_name};
use super::{CommandSpec, ok};
use crate::params::{boolean, get, id, num, req_id, req_num, req_string, req_targets, string, targets};
use crate::{Result, Session, bad, spec, textedit};

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// Apply a style change to the edited range, or to whole text layers in the selection.
fn style(s: &mut Session, p: &Value, f: impl Fn(&mut TextStyle)) -> Result<Value> {
    if let Some((id, a, b)) = textedit::selected_range(s) {
        let ds = s.ds_mut()?;
        let before = ds.doc.clone();
        let doc = ds.doc_mut();
        let bound = super::styles::bound_before(doc, &[id]);
        if let NodeKind::Text(t) = &mut doc.get_mut(id)?.kind {
            if a == b {
                t.restyle(0, t.char_count(), &f);
            } else {
                t.restyle(a, b, &f);
            }
        }
        super::styles::detach_edited(doc, &bound);
        crate::normalize(doc, &before);
        return ok();
    }
    let ids = req_targets("text", s, p)?;
    let doc = s.doc_mut()?;
    let ids: Vec<NodeId> = ids.into_iter().flat_map(|i| doc.subtree(i)).collect();
    let before = super::styles::bound_before(doc, &ids);
    for d in ids {
        if let Some(NodeKind::Text(t)) = doc.node_mut(d).map(|n| &mut n.kind) {
            let n = t.char_count();
            t.restyle(0, n, &f);
        }
    }
    super::styles::detach_edited(doc, &before);
    ok()
}

fn para(s: &mut Session, p: &Value, f: impl Fn(&mut TextData)) -> Result<Value> {
    let ids = match textedit::selected_range(s) {
        Some((id, _, _)) => vec![id],
        None => req_targets("text", s, p)?,
    };
    let ds = s.ds_mut()?;
    let before = ds.doc.clone();
    let doc = ds.doc_mut();
    for i in ids {
        for d in doc.subtree(i) {
            if let Some(NodeKind::Text(t)) = doc.node_mut(d).map(|n| &mut n.kind) {
                f(t);
            }
        }
    }
    crate::normalize(doc, &before);
    ok()
}

/// Run `f` on each targeted text with the chars whose paragraphs a list command changes: the
/// edited paragraphs while editing, else all of them.
fn para_range(s: &mut Session, p: &Value, f: impl Fn(&mut TextData, usize, usize)) -> Result<Value> {
    let range = textedit::selected_range(s);
    para(s, p, |t| {
        let (a, b) = range.map_or((0, t.char_count()), |(_, a, b)| (a, b));
        f(t, a, b)
    })
}

/// A list command: every targeted paragraph becomes `l`, or (`toggle`) plain when all already are.
fn set_list(s: &mut Session, p: &Value, l: ListStyle, toggle: bool) -> Result<Value> {
    let range = textedit::selected_range(s);
    let ids = range.map_or_else(|| targets(s, p), |(id, _, _)| vec![id]);
    let doc = s.doc()?;
    let all = ids.iter().all(|i| {
        doc.node(*i).and_then(|n| n.kind.text()).is_some_and(|t| {
            let (a, b) = range.map_or((0, t.char_count()), |(_, a, b)| (a, b));
            t.common_para_list(a, b).is_some_and(|pl| pl.list == l)
        })
    });
    let l = if toggle && all { ListStyle::None } else { l };
    para_range(s, p, |t, a, b| t.set_para_lists(a, b, |pl| pl.list = l))
}

/// Whether every targeted char has `pred` (to decide toggle direction).
fn all_have(s: &Session, p: &Value, pred: impl Fn(&TextStyle) -> bool) -> bool {
    let Ok(doc) = s.doc() else { return false };
    if let Some((id, a, b)) = textedit::selected_range(s) {
        if let Some(NodeKind::Text(t)) = doc.node(id).map(|n| &n.kind) {
            let (a, b) = if a == b { (0, t.char_count()) } else { (a, b) };
            return (a..b.max(a + 1)).all(|i| pred(t.style_at(i)));
        }
        return false;
    }
    targets(s, p).iter().all(|i| matches!(doc.node(*i).map(|n| &n.kind), Some(NodeKind::Text(t)) if t.spans().iter().all(|(_, _, st)| pred(st))))
}

fn toggle_bold(s: &mut Session, p: &Value) -> Result<Value> {
    let bold = all_have(s, p, |st| st.weight() >= 700);
    style(s, p, |st| {
        let italic = st.italic();
        st.set_font_style(styled_name(if bold { "Regular" } else { "Bold" }, italic));
    })
}

fn toggle_italic(s: &mut Session, p: &Value) -> Result<Value> {
    let italic = all_have(s, p, TextStyle::italic);
    style(s, p, |st| {
        let w = weight_name(st.weight());
        st.set_font_style(styled_name(w, !italic));
    })
}

fn step_weight(s: &mut Session, p: &Value, up: bool) -> Result<Value> {
    style(s, p, |st| {
        let w = (i32::from(st.weight()) + if up { 100 } else { -100 }).clamp(100, 900) as u16;
        let italic = st.italic();
        st.set_font_style(styled_name(weight_name(w), italic));
    })
}

/// Put a text layer on a path: the text takes the path layer's place, geometry and transform, and
/// the path layer is removed. The path's effects move to the text when it has any, and its fills
/// when one of them shows, each with its style and variable bindings; otherwise the text keeps
/// its own.
fn on_path(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "text.onPath";
    let doc = s.doc()?;
    let is_text = |i: &NodeId| matches!(doc.node(*i).map(|n| &n.kind), Some(NodeKind::Text(_)));
    let sel = targets(s, p);
    let text =
        id(p, "text").or_else(|| sel.iter().copied().find(is_text)).ok_or_else(|| bad(CMD, "select a text layer and a path (or pass `text`)"))?;
    let path = id(p, "path")
        .or_else(|| sel.iter().copied().find(|i| *i != text && !is_text(i)))
        .ok_or_else(|| bad(CMD, "select a path or shape too (or pass `path`)"))?;
    let pn = doc.get(path)?;
    let geom = figcraft_doc::geometry::stroke_geometry(pn).filter(|g| !g.elements().is_empty()).ok_or_else(|| bad(CMD, "that layer has no path"))?;
    if !is_text(&text) || doc.is_ancestor(path, text) {
        return Err(bad(CMD, "`text` must be a text layer outside the path layer"));
    }
    let (parent, index) = (pn.parent.ok_or_else(|| bad(CMD, "pages have no path"))?, doc.index_in_parent(path));
    let (transform, w, h) = (pn.transform, pn.width, pn.height);
    let fills = pn.fills.iter().any(|f| f.visible).then(|| (pn.fills.clone(), pn.styles.fill.clone()));
    let effects = (!pn.effects.is_empty()).then(|| (pn.effects.clone(), pn.styles.effect.clone()));
    let moved = |k: &str| match super::lists::list_binding(k) {
        Some((super::lists::BoundList::Fills, ..)) => fills.is_some(),
        Some((super::lists::BoundList::Effects, ..)) => effects.is_some(),
        _ => false,
    };
    let bound: Vec<(String, VariableId)> = pn.bound.iter().filter(|(k, _)| moved(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut tp = TextPath::new(&geom);
    tp.start = num(p, "start").filter(|v| v.is_finite()).unwrap_or(0.0).clamp(0.0, 1.0);
    tp.flip = boolean(p, "flip").unwrap_or(false);
    let doc = s.doc_mut()?;
    doc.reparent(text, parent, index)?;
    doc.remove(path)?;
    let n = doc.get_mut(text)?;
    n.transform = transform;
    (n.width, n.height) = (w, h);
    n.bound.retain(|k, _| !moved(k));
    n.bound.extend(bound);
    if let Some((fills, style)) = fills {
        (n.fills, n.styles.fill) = (fills, style);
    }
    if let Some((effects, style)) = effects {
        (n.effects, n.styles.effect) = (effects, style);
    }
    if let NodeKind::Text(t) = &mut n.kind {
        t.path = Some(Box::new(tp));
    }
    s.set_selection(vec![text]);
    Ok(json!({ "id": text }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "text.onPath",
            "Text on path",
            "{text? (text layer), path? (vector or shape layer; default: the selected text and other layer), start? (0..1 along the path), flip?}  the text follows the path and takes its effects and its fills when one shows, and the path layer is removed → {id}",
            true,
            on_path
        ),
        spec!("text.pathOptions", "Text path options", "{start? (0..1 along the path), flip?, ids?}  for text on a path", true, |s, p| {
            let start = num(p, "start").filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0));
            let flip = boolean(p, "flip");
            para(s, p, |t| {
                if let Some(tp) = &mut t.path {
                    tp.start = start.unwrap_or(tp.start);
                    tp.flip = flip.unwrap_or(tp.flip);
                }
            })
        }),
        spec!("text.removeFromPath", "Remove text from path", "{ids?}  text on a path back to a line of auto-width text", true, |s, p| {
            para(s, p, |t| {
                if t.path.take().is_some() {
                    t.auto_resize = TextAutoResize::WidthAndHeight;
                }
            })
        }),
        spec!("text.bold", "Bold", "{ids?}", true, toggle_bold),
        spec!("text.italic", "Italic", "{ids?}", true, toggle_italic),
        spec!("text.underline", "Underline", "{ids?}", true, |s, p| {
            let on = all_have(s, p, |st| st.decoration == TextDecoration::Underline);
            style(s, p, |st| st.decoration = if on { TextDecoration::None } else { TextDecoration::Underline })
        }),
        spec!("text.strikethrough", "Strikethrough", "{ids?}", true, |s, p| {
            let on = all_have(s, p, |st| st.decoration == TextDecoration::Strikethrough);
            style(s, p, |st| st.decoration = if on { TextDecoration::None } else { TextDecoration::Strikethrough })
        }),
        spec!("text.link", "Create link", "{url, ids?}", true, |s, p| {
            let url = req_string("text.link", p, "url")?;
            style(s, p, |st| st.link = Some(url.clone()))
        }),
        spec!("text.bulletList", "Bulleted list", "{ids?}  while editing: the selected paragraphs", true, |s, p| set_list(
            s,
            p,
            ListStyle::Bulleted,
            true
        )),
        spec!("text.numberList", "Numbered list", "{ids?}  while editing: the selected paragraphs", true, |s, p| set_list(
            s,
            p,
            ListStyle::Numbered,
            true
        )),
        spec!("text.alignLeft", "Align left", "{ids?}", true, |s, p| para(s, p, |t| t.align_h = TextAlignH::Left)),
        spec!("text.alignCenter", "Align center", "{ids?}", true, |s, p| para(s, p, |t| t.align_h = TextAlignH::Center)),
        spec!("text.alignRight", "Align right", "{ids?}", true, |s, p| para(s, p, |t| t.align_h = TextAlignH::Right)),
        spec!("text.alignJustify", "Justify", "{ids?}", true, |s, p| para(s, p, |t| t.align_h = TextAlignH::Justified)),
        spec!("text.sizeUp", "Increase font size", "{ids?}", true, |s, p| style(s, p, |st| st.font_size = (st.font_size + 1.0).min(5000.0))),
        spec!("text.sizeDown", "Decrease font size", "{ids?}", true, |s, p| style(s, p, |st| st.font_size = (st.font_size - 1.0).max(1.0))),
        spec!("text.weightUp", "Increase font weight", "{ids?}", true, |s, p| step_weight(s, p, true)),
        spec!("text.weightDown", "Decrease font weight", "{ids?}", true, |s, p| step_weight(s, p, false)),
        spec!("text.lineHeightUp", "Increase line height", "{ids?}", true, |s, p| {
            style(s, p, |st| st.line_height = LineHeight::Pixels(st.line_height.resolve(st.font_size, 1.21).round() + 1.0))
        }),
        spec!("text.lineHeightDown", "Decrease line height", "{ids?}", true, |s, p| {
            style(s, p, |st| st.line_height = LineHeight::Pixels((st.line_height.resolve(st.font_size, 1.21).round() - 1.0).max(0.0)))
        }),
        spec!("text.trackingUp", "Increase letter spacing", "{ids?}", true, |s, p| {
            style(s, p, |st| {
                st.letter_spacing = LetterSpacing::Percent(match st.letter_spacing {
                    LetterSpacing::Percent(v) => v + 1.0,
                    LetterSpacing::Pixels(v) => v / st.font_size.max(1.0) * 100.0 + 1.0,
                })
            })
        }),
        spec!("text.trackingDown", "Decrease letter spacing", "{ids?}", true, |s, p| {
            style(s, p, |st| {
                st.letter_spacing = LetterSpacing::Percent(match st.letter_spacing {
                    LetterSpacing::Percent(v) => v - 1.0,
                    LetterSpacing::Pixels(v) => v / st.font_size.max(1.0) * 100.0 - 1.0,
                })
            })
        }),
        spec!("text.caseOriginal", "Original case", "{ids?}", true, |s, p| style(s, p, |st| st.case = TextCase::Original)),
        spec!("text.caseUpper", "Uppercase", "{ids?}", true, |s, p| style(s, p, |st| st.case = TextCase::Upper)),
        spec!("text.caseLower", "Lowercase", "{ids?}", true, |s, p| style(s, p, |st| st.case = TextCase::Lower)),
        spec!("text.caseTitle", "Title case", "{ids?}", true, |s, p| style(s, p, |st| st.case = TextCase::Title)),
        spec!("text.caseSmallCaps", "Small caps", "{ids?}", true, |s, p| style(s, p, |st| st.case = TextCase::SmallCaps)),
        spec!("text.case", "Letter case", "{case: original|upper|lower|title|smallCaps, ids?}", true, |s, p| {
            let c: TextCase =
                get(p, "case").and_then(super::props::enum_of).ok_or_else(|| bad("text.case", "`case`: original|upper|lower|title|smallCaps"))?;
            style(s, p, |st| st.case = c)
        }),
        spec!("text.decoration", "Decoration", "{decoration: none|underline|strikethrough, ids?}", true, |s, p| {
            let d: TextDecoration = get(p, "decoration")
                .and_then(super::props::enum_of)
                .ok_or_else(|| bad("text.decoration", "`decoration`: none|underline|strikethrough"))?;
            style(s, p, |st| st.decoration = d)
        }),
        spec!(
            "text.listStyle",
            "List style",
            "{style | list: none|bulleted|numbered, ids?}  while editing: the selected paragraphs",
            true,
            |s, p| {
                let l: ListStyle = get(p, "style")
                    .or_else(|| get(p, "list"))
                    .and_then(super::props::enum_of)
                    .ok_or_else(|| bad("text.listStyle", "`style`: none|bulleted|numbered"))?;
                set_list(s, p, l, false)
            }
        ),
        spec!("text.paragraphIndent", "Paragraph indent", "{indent (px), ids?}", true, |s, p| {
            let v = finite(req_num("text.paragraphIndent", p, "indent")?).clamp(0.0, 10_000.0);
            para(s, p, |t| t.paragraph_indent = v)
        }),
        spec!("text.paragraphSpacing", "Paragraph spacing", "{spacing (px), ids?}", true, |s, p| {
            let v = finite(req_num("text.paragraphSpacing", p, "spacing")?).clamp(0.0, 10_000.0);
            para(s, p, |t| t.paragraph_spacing = v)
        }),
        spec!(
            "text.maxLines",
            "Max lines",
            "{lines (0 = no limit; truncates: auto-height text hugs up to that many lines, other text becomes a truncating box), ids?}",
            true,
            |s, p| {
                let v = finite(req_num("text.maxLines", p, "lines")?).clamp(0.0, 10_000.0) as u32;
                para(s, p, |t| {
                    t.max_lines = v;
                    if v > 0 && t.auto_resize != TextAutoResize::Height {
                        t.auto_resize = TextAutoResize::Truncate;
                    }
                })
            }
        ),
        spec!(
            "text.setFeatures",
            "OpenType features",
            "{features: {tag: bool | null (font default)}, replace? (clear others first), id?/ids?}",
            true,
            |s, p| {
                let f = super::props::features_of("text.setFeatures", get(p, "features").unwrap_or(&Value::Null))?;
                let replace = boolean(p, "replace").unwrap_or(false);
                para(s, p, |t| {
                    if replace {
                        t.features.clear();
                    }
                    for (k, on) in &f {
                        match on {
                            Some(on) => {
                                t.features.insert(k.clone(), *on);
                            }
                            None => {
                                t.features.remove(k);
                            }
                        }
                    }
                })
            }
        ),
        spec!(
            "text.setVariations",
            "Variable font axes",
            "{variations: {axis: number | null (style default)}, replace? (clear others first), id?/ids?}; while editing: the selected range",
            true,
            |s, p| {
                let vars = super::props::variations_of("text.setVariations", get(p, "variations").unwrap_or(&Value::Bool(false)))?;
                let replace = boolean(p, "replace").unwrap_or(false);
                style(s, p, |st| {
                    if replace {
                        st.variations.clear();
                    }
                    for (k, v) in &vars {
                        match v {
                            Some(v) => {
                                st.variations.insert(k.clone(), *v);
                            }
                            None => {
                                st.variations.remove(k);
                            }
                        }
                    }
                })
            }
        ),
        spec!(
            "text.preview",
            "Preview a change",
            "{command: style.apply|library.applyStyle|text.setVariations|text.style|node.set|fill.set|stroke.set|selection.setPaint|style.update, params}  shows what the command would do on the canvas without an undo step (a hovered font family or style, text style, named instance, blend mode, or a dragged axis slider), in place of an earlier preview; text.endPreview or any other command ends it, so running the command for real commits it as one undo step",
            false,
            |s, p| {
                let c = req_string("text.preview", p, "command")?;
                let params = get(p, "params").cloned().unwrap_or_else(|| json!({}));
                crate::textpreview::show(s, &c, &params)?;
                Ok(json!({"previewing": crate::textpreview::active(s)}))
            }
        ),
        spec!("text.endPreview", "End text preview", "{}  puts the document back as it was before text.preview → {ended}", false, |s, _| {
            Ok(json!({"ended": crate::textpreview::end(s)}))
        }),
        spec!("text.set", "Set text", "{id?, characters | text}", true, |s, p| {
            let c = string(p, "characters").or_else(|| string(p, "text")).ok_or_else(|| bad("text.set", "`characters` required"))?;
            let ids = req_targets("text.set", s, p)?;
            let doc = s.doc_mut()?;
            let before = super::styles::bound_before(doc, &ids);
            for i in ids {
                let mut m = Map::new();
                m.insert("characters".into(), json!(c));
                super::props::apply("text.set", doc, i, &m)?;
            }
            super::styles::detach_edited(doc, &before);
            ok()
        }),
        spec!(
            "text.style",
            "Style text range",
            "{id?, start?, end?, fontFamily?, fontStyle?, fontWeight?, fontVariations? | variationSettings?, fontSize?, lineHeight?, letterSpacing?, textDecoration?, textDecorationStyle?, textDecorationThickness?, textDecorationOffset?, textDecorationSkipInk?, textDecorationColor?, textCase?, hyperlink?, textFills?}",
            true,
            |s, p| {
                let ids = req_targets("text.style", s, p)?;
                let m = p.as_object().cloned().unwrap_or_default();
                let doc = s.doc_mut()?;
                let before = super::styles::bound_before(doc, &ids);
                for i in ids {
                    let n = doc.get_mut(i)?;
                    let NodeKind::Text(t) = &mut n.kind else { return Err(bad("text.style", format!("{i} is not text"))) };
                    let len = t.char_count();
                    let a = num(p, "start").map(|v| v.max(0.0) as usize).unwrap_or(0).min(len);
                    let b = num(p, "end").map(|v| v.max(0.0) as usize).unwrap_or(len).min(len);
                    for (k, v) in &m {
                        if matches!(k.as_str(), "id" | "ids" | "start" | "end") {
                            continue;
                        }
                        super::props::apply_text_style("text.style", t, a, b, k, v)?;
                    }
                }
                super::styles::detach_edited(doc, &before);
                ok()
            }
        ),
        spec!("text.edit.begin", "Edit text", "{id, caret? (char index; default select all)}", false, |s, p| {
            let id = req_id("text.edit.begin", p, "id")?;
            let caret = num(p, "caret").map(|c| c.max(0.0) as usize);
            textedit::begin(s, id, caret, false)?;
            ok()
        }),
        spec!("text.edit.insert", "Type text", "{text}", true, |s, p| {
            let t = string(p, "text").unwrap_or_default();
            textedit::insert(s, &t)?;
            ok()
        }),
        spec!(
            "text.edit.key",
            "Text key",
            "{key: Backspace|Delete|Left|Right|Up|Down|Home|End|Enter|Tab|Escape|SelectAll, shift?, alt?, cmd?}  in a list, Tab nests the item and Shift+Tab, Enter on an empty item or Backspace at its start move it out",
            true,
            |s, p| {
                let k = req_string("text.edit.key", p, "key")?;
                textedit::key(s, &k, boolean(p, "shift").unwrap_or(false), boolean(p, "alt").unwrap_or(false), boolean(p, "cmd").unwrap_or(false))?;
                ok()
            }
        ),
        spec!("text.listIndent", "Indent list", "{}  while editing: the selected list items a level deeper", true, |s, _| {
            Ok(json!({"changed": textedit::list_indent(s, false)?}))
        }),
        spec!("text.listOutdent", "Outdent list", "{}  while editing: the selected list items a level out", true, |s, _| {
            Ok(json!({"changed": textedit::list_indent(s, true)?}))
        }),
        spec!("text.edit.caret", "Set caret", "{index, extend?}", false, |s, p| {
            let i = req_num("text.edit.caret", p, "index")?.max(0.0) as usize;
            textedit::set_caret(s, i, boolean(p, "extend").unwrap_or(false))?;
            ok()
        }),
        spec!("text.edit.selectWord", "Select word", "{index}", false, |s, p| {
            let i = finite(req_num("text.edit.selectWord", p, "index")?).max(0.0) as usize;
            textedit::select_word(s, i)?;
            ok()
        }),
        spec!("text.edit.selectLine", "Select line", "{index}", false, |s, p| {
            let i = finite(req_num("text.edit.selectLine", p, "index")?).max(0.0) as usize;
            textedit::select_line(s, i)?;
            ok()
        }),
        spec!(
            "text.edit.preedit",
            "Compose text (IME)",
            "{text (empty = cancel), cursor? (chars into text)}; shown at the caret, committed by text.edit.insert",
            false,
            |s, p| {
                let cursor = num(p, "cursor").map(|c| c.max(0.0) as usize);
                textedit::preedit(s, &string(p, "text").unwrap_or_default(), cursor)?;
                ok()
            }
        ),
        spec!("text.edit.selection", "Text selection", "{}", false, |s, _| {
            Ok(match s.ds()?.text_edit.clone() {
                Some(te) => json!({
                    "id": te.id, "caret": te.caret, "anchor": te.anchor, "text": textedit::selected_text(s),
                    "preedit": te.preedit.map(|p| json!({"text": p.text, "cursor": p.cursor})),
                }),
                None => Value::Null,
            })
        }),
        spec!("text.edit.end", "Stop editing text", "{}", false, |s, _| {
            textedit::end(s)?;
            ok()
        }),
        spec!(
            "text.fonts",
            "List fonts",
            "{family? (list its styles), style?, features? (the face's OpenType features), axes? (the face's variation axes and named instances)}",
            false,
            |_, p| {
                let face = |f: &str| figcraft_text::find_face(f, &string(p, "style").unwrap_or_else(|| "Regular".into()));
                Ok(match string(p, "family") {
                    Some(f) if boolean(p, "features").unwrap_or(false) => json!(face(&f).map(|face| face.feature_tags()).unwrap_or_default()),
                    Some(f) if boolean(p, "axes").unwrap_or(false) => match face(&f) {
                        Some(face) => json!({
                            "family": face.family,
                            "axes": face.axes.iter().map(|a| json!({
                                "tag": a.tag, "name": a.name, "min": a.min, "default": a.default, "max": a.max, "hidden": a.hidden
                            })).collect::<Vec<_>>(),
                            "instances": face.instances.iter().map(|i| json!({
                                "name": i.name,
                                "variations": face.axes.iter().zip(&i.coords).map(|(a, c)| (a.tag.clone(), json!(c))).collect::<Map<String, Value>>()
                            })).collect::<Vec<_>>(),
                        }),
                        None => Value::Null,
                    },
                    Some(f) => json!(figcraft_text::family_styles(&f)),
                    None => json!(figcraft_text::family_names()),
                })
            }
        ),
        spec!(
            "text.rescanFonts",
            "Rescan installed fonts",
            "{}  walks the system font folders again, for fonts installed while the app runs; the open document's auto-sizing text in the families that gained faces (new ones, or styles installed since) fits them (text.refit) → {added: [families that were unknown], refitted: [ids]}",
            true,
            |s, _| {
                let rescan = figcraft_text::rescan_system_fonts();
                let refitted = crate::fonts::refit_open(s, None, Some(&rescan.changed));
                Ok(json!({"added": rescan.added, "refitted": refitted}))
            }
        ),
        spec!(
            "text.loadFont",
            "Load font file",
            "{path | data (base64)}  adds a font file (.ttf, .otf, .ttc, .otc; every face of a collection) to this session's fonts without installing it; the open document's auto-sizing text in its families fits it (text.refit) → {families, refitted: [ids]}",
            true,
            |s, p| {
                let cmd = "text.loadFont";
                let bytes = if let Some(b64) = string(p, "data") {
                    use base64::Engine as _;
                    let b64 = b64.split_once(',').map(|(_, d)| d.to_string()).unwrap_or(b64);
                    base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| bad(cmd, format!("`data`: {e}")))?
                } else if let Some(path) = string(p, "path") {
                    super::file::read_capped(&path, figcraft_text::fontdb::MAX_FONT_FILE)?
                } else {
                    return Err(bad(cmd, "pass `path` or `data` (base64)"));
                };
                let families = figcraft_text::load_font_bytes(bytes);
                if families.is_empty() {
                    return Err(bad(cmd, "not a font file"));
                }
                let refitted = crate::fonts::refit_open(s, None, Some(&families));
                Ok(json!({"families": families, "refitted": refitted}))
            }
        ),
    ]
}
