use super::paint::color;
use super::{Kv, MAX_EXPR_DEPTH, Mapper, Obj, Value, View, guid, json, num, ref_guid};

impl<'a> Mapper<'a> {
    pub(super) fn variable_bindings(&mut self, o: &mut Obj, v: &View<'a>) {
        let mut bound = Obj::new();
        let mut text_bound = Obj::new();
        for e in v.get("variableConsumptionMap").and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default() {
            let Some(var) = alias(e.get("variableData")) else { continue };
            let a = json!({"type": "VARIABLE_ALIAS", "id": var});
            let field = e.get("variableField").and_then(Kv::str).unwrap_or("");
            match (binding_key(field), text_binding_key(field)) {
                (Some(k), _) => {
                    bound.insert(k.into(), a);
                }
                (_, Some(k)) => {
                    text_bound.insert(k.into(), a);
                }
                _ if field == "CORNER_RADIUS" => {
                    for k in ["topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius"] {
                        bound.insert(k.into(), a.clone());
                    }
                }
                _ if field.starts_with("BORDER_") => {
                    let side = match field {
                        "BORDER_TOP_WEIGHT" => "strokeTopWeight",
                        "BORDER_RIGHT_WEIGHT" => "strokeRightWeight",
                        "BORDER_BOTTOM_WEIGHT" => "strokeBottomWeight",
                        _ => "strokeLeftWeight",
                    };
                    bound.insert(side.into(), a);
                }
                _ if field == "WIDTH" || field == "HEIGHT" => {
                    let axis = if field == "WIDTH" { "x" } else { "y" };
                    let size = bound.entry("size").or_insert_with(|| json!({}));
                    size[axis] = a;
                }
                _ => self.warn(format!("variables bound to {} are not imported from .fig files yet", field.to_ascii_lowercase().replace('_', " "))),
            }
        }
        if !bound.is_empty() {
            o.insert("boundVariables".into(), Value::Object(bound));
        }
        // (The text style itself is added after this: it keeps these.)
        if !text_bound.is_empty() {
            o.insert("style".into(), json!({"boundVariables": text_bound}));
        }
        let mut modes = Obj::new();
        let mut lost = false;
        for e in v.get("variableModeBySetMap").and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default() {
            let (Some(set), Some(mode)) = (e.get("variableSetID"), e.get("variableModeID").and_then(guid)) else { continue };
            match self.resource_id(set) {
                Some(id) => {
                    modes.insert(id, json!(mode));
                }
                // A mode of a collection neither named by guid nor resolvable in the file.
                None if set.get("guid").is_some() || set.get("assetRef").is_some() => lost = true,
                None => {}
            }
        }
        if lost {
            self.warn("variable modes of collections that aren't in the file were skipped");
        }
        if !modes.is_empty() {
            o.insert("explicitVariableModes".into(), Value::Object(modes));
        }
    }
}

/// The REST `boundVariables` key of a bound layer field.
fn binding_key(field: &str) -> Option<&'static str> {
    Some(match field {
        "STROKE_WEIGHT" => "strokeWeight",
        "STACK_SPACING" => "itemSpacing",
        "STACK_COUNTER_SPACING" => "counterAxisSpacing",
        "STACK_PADDING_LEFT" => "paddingLeft",
        "STACK_PADDING_TOP" => "paddingTop",
        "STACK_PADDING_RIGHT" => "paddingRight",
        "STACK_PADDING_BOTTOM" => "paddingBottom",
        "VISIBLE" => "visible",
        "TEXT_DATA" => "characters",
        "OPACITY" => "opacity",
        "MIN_WIDTH" => "minWidth",
        "MAX_WIDTH" => "maxWidth",
        "MIN_HEIGHT" => "minHeight",
        "MAX_HEIGHT" => "maxHeight",
        "RECTANGLE_TOP_LEFT_CORNER_RADIUS" => "topLeftRadius",
        "RECTANGLE_TOP_RIGHT_CORNER_RADIUS" => "topRightRadius",
        "RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS" => "bottomLeftRadius",
        "RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS" => "bottomRightRadius",
        _ => return None,
    })
}

/// The REST text style `boundVariables` key of a bound text field.
fn text_binding_key(field: &str) -> Option<&'static str> {
    Some(match field {
        "FONT_SIZE" => "fontSize",
        "FONT_FAMILY" => "fontFamily",
        "FONT_STYLE" => "fontStyle",
        "LETTER_SPACING" => "letterSpacing",
        "LINE_HEIGHT" => "lineHeight",
        "PARAGRAPH_SPACING" => "paragraphSpacing",
        "PARAGRAPH_INDENT" => "paragraphIndent",
        _ => return None,
    })
}

/// A `VariableData` (literal, variable reference or expression) in the REST shape; `None` for
/// kinds a prototype can't evaluate.
pub(super) fn variable_data(d: &Kv, depth: usize) -> Option<Value> {
    let v = d.get("value")?;
    let ty = d.get("dataType").and_then(Kv::str)?;
    let value = match ty {
        "ALIAS" => json!({"type": "VARIABLE_ALIAS", "id": v.get("alias").and_then(ref_guid)?}),
        "EXPRESSION" if depth < MAX_EXPR_DEPTH => {
            let e = v.get("expressionValue")?;
            let function = match e.get("expressionFunction").and_then(Kv::str)? {
                "MULTIPLY" => "MULTIPLICATION",
                "DIVIDE" => "DIVISION",
                f => f,
            };
            let args = e.get("expressionArguments").map(Kv::list).unwrap_or_default();
            let args = args.iter().map(|x| variable_data(x, depth + 1)).collect::<Option<Vec<Value>>>()?;
            json!({"expressionFunction": function, "expressionArguments": args})
        }
        "BOOLEAN" => json!(v.get("boolValue").and_then(Kv::bool)?),
        "FLOAT" => json!(num(v, "floatValue").filter(|x| x.is_finite())?),
        "STRING" => json!(v.get("textValue").and_then(Kv::str)?),
        "COLOR" => color(v.get("colorValue")?),
        _ => return None,
    };
    let mut out = json!({"type": ty, "value": value});
    if let Some(r) = d.get("resolvedDataType").and_then(Kv::str) {
        out["resolvedType"] = json!(r);
    }
    Some(out)
}

/// The variable an `ALIAS` variable data points at.
pub(super) fn alias(d: Option<&Kv>) -> Option<String> {
    let d = d?;
    if d.get("dataType").and_then(Kv::str) != Some("ALIAS") {
        return None;
    }
    d.get("value").and_then(|v| v.get("alias")).and_then(ref_guid)
}

pub(super) fn variable_value(d: &Kv) -> Option<Value> {
    if let Some(a) = alias(Some(d)) {
        return Some(json!({"type": "VARIABLE_ALIAS", "id": a}));
    }
    let v = d.get("value")?;
    if let Some(c) = v.get("colorValue") {
        return Some(color(c));
    }
    if let Some(b) = v.get("boolValue").and_then(Kv::bool) {
        return Some(json!(b));
    }
    if let Some(x) = num(v, "floatValue") {
        return Some(json!(x));
    }
    v.get("textValue").and_then(Kv::str).map(|s| json!(s))
}
