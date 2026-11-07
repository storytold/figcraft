use super::paint::{bound, color};
use super::{BTreeMap, BTreeSet, Kv, MAX_GRID_TRACK_VALUE, MAX_GRID_TRACKS, Mapper, Obj, Value, View, copy_num, guid, json, num};

impl<'a> Mapper<'a> {
    pub(super) fn auto_layout(&mut self, o: &mut Obj, v: &View<'a>) {
        let Some(mode) = v.str("stackMode").filter(|m| matches!(*m, "HORIZONTAL" | "VERTICAL" | "GRID")) else { return };
        o.insert("layoutMode".into(), json!(mode));
        copy_num(o, v, "stackSpacing", "itemSpacing");
        copy_num(o, v, "stackCounterSpacing", "counterAxisSpacing");
        if v.str("stackWrap") == Some("WRAP") {
            o.insert("layoutWrap".into(), json!("WRAP"));
        }
        if v.str("stackCounterAlignContent") == Some("SPACE_BETWEEN") {
            o.insert("counterAxisAlignContent".into(), json!("SPACE_BETWEEN"));
        }
        let (h, vert) = (v.num("stackHorizontalPadding").unwrap_or(0.0), v.num("stackVerticalPadding").unwrap_or(0.0));
        o.insert("paddingLeft".into(), json!(h));
        o.insert("paddingTop".into(), json!(vert));
        o.insert("paddingRight".into(), json!(v.num("stackPaddingRight").unwrap_or(h)));
        o.insert("paddingBottom".into(), json!(v.num("stackPaddingBottom").unwrap_or(vert)));
        let main = match v.str("stackPrimaryAlignItems").or_else(|| v.str("stackJustify")) {
            Some("CENTER") => "CENTER",
            Some("MAX") => "MAX",
            Some("SPACE_AROUND") => "SPACE_AROUND",
            // `SPACE_EVENLY` is the format's older name for equal gaps without edge space.
            Some("SPACE_EVENLY_CSS") => "SPACE_EVENLY",
            Some("SPACE_BETWEEN" | "SPACE_EVENLY") => "SPACE_BETWEEN",
            _ => "MIN",
        };
        o.insert("primaryAxisAlignItems".into(), json!(main));
        let cross = match v.str("stackCounterAlignItems").or_else(|| v.str("stackAlign")) {
            Some("CENTER") => "CENTER",
            Some("MAX") => "MAX",
            Some("BASELINE") => "BASELINE",
            _ => "MIN",
        };
        o.insert("counterAxisAlignItems".into(), json!(cross));
        let sizing = |k: &str| if v.str(k).is_some_and(|s| s.starts_with("RESIZE_TO_FIT")) { "AUTO" } else { "FIXED" };
        // A flow that records no sizing for its primary axis sizes it by its content: auto layout
        // did before that axis could be fixed.
        let primary = if v.get("stackPrimarySizing").is_none() && mode != "GRID" { "AUTO" } else { sizing("stackPrimarySizing") };
        o.insert("primaryAxisSizingMode".into(), json!(primary));
        o.insert("counterAxisSizingMode".into(), json!(sizing("stackCounterSizing")));
        if let Some(b) = v.flag("bordersTakeSpace") {
            o.insert("strokesIncludedInLayout".into(), json!(b));
        }
        if let Some(b) = v.flag("stackReverseZIndex") {
            o.insert("itemReverseZIndex".into(), json!(b));
        }
        if mode == "GRID" {
            copy_num(o, v, "gridColumnGap", "gridColumnGap");
            copy_num(o, v, "gridRowGap", "gridRowGap");
            o.insert("gridColumnCount".into(), json!(tracks(v, "gridColumns").len()));
            o.insert("gridRowCount".into(), json!(tracks(v, "gridRows").len()));
            let track_data = [("columnTracks", "gridColumns", "gridColumnsSizing"), ("rowTracks", "gridRows", "gridRowsSizing")]
                .into_iter()
                .filter_map(|(out_key, order_key, sizing_key)| {
                    self.map_grid_tracks(v, order_key, sizing_key).map(|mapped| (out_key.to_string(), mapped))
                })
                .collect::<Obj>();
            if !track_data.is_empty() {
                let source = o.entry(crate::rest::FIG_SOURCE_FIELD).or_insert_with(|| json!({}));
                if let Some(source) = source.as_object_mut() {
                    source.insert("gridTracks".into(), Value::Object(track_data));
                }
            }
        }
    }

    /// Track sizing maps are keyed by GUID; their order comes from the separate track list.
    fn map_grid_tracks(&mut self, v: &View<'a>, order_key: &str, sizing_key: &str) -> Option<Value> {
        let track_entries = v.get(order_key).and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default();
        let sizes = v.get(sizing_key).and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default();
        if track_entries.len() > MAX_GRID_TRACKS {
            self.warn("grid track list exceeds 1000 entries; the tracks use the default equal sizing");
            return None;
        }
        if sizes.is_empty() {
            return None;
        }
        if sizes.len() > MAX_GRID_TRACKS {
            self.warn("grid track sizing list exceeds 1000 entries; the tracks use the default equal sizing");
            return None;
        }
        let ordered = tracks(v, order_key);
        let ordered_ids: BTreeSet<&str> = ordered.iter().map(String::as_str).collect();
        let mut size_by_id: BTreeMap<String, Option<&Kv<'a>>> = BTreeMap::new();
        for entry in sizes {
            let Some(id) = entry.get("id").and_then(guid) else {
                self.warn("stale or invalid grid track sizing entries were ignored");
                continue;
            };
            if !ordered_ids.contains(id.as_str()) {
                self.warn("stale or invalid grid track sizing entries were ignored");
                continue;
            }
            match size_by_id.entry(id) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(Some(entry));
                }
                std::collections::btree_map::Entry::Occupied(mut slot) => {
                    slot.insert(None);
                    self.warn("duplicate grid track sizing entries use equal tracks");
                }
            }
        }
        let mut mapped = Vec::with_capacity(ordered.len());
        for id in ordered {
            if let Some(Some(entry)) = size_by_id.get(&id) {
                match grid_track_sizing(entry.get("trackSize")) {
                    Some(track) => mapped.push(track),
                    None => {
                        self.warn("grid track sizing is unsupported or malformed; affected tracks use equal sizing");
                        mapped.push(json!({"sizing": "fill", "value": 1.0}));
                    }
                }
            } else {
                mapped.push(json!({"sizing": "fill", "value": 1.0}));
            }
        }
        Some(Value::Array(mapped))
    }
}

/// The track ids of a grid's columns or rows, in order.
pub(super) fn tracks(v: &View, key: &str) -> Vec<String> {
    let mut t: Vec<(&str, String)> = v
        .get(key)
        .and_then(|m| m.get("entries"))
        .map(Kv::list)
        .unwrap_or_default()
        .iter()
        .filter_map(|e| Some((e.get("position").and_then(Kv::str).unwrap_or(""), e.get("id").and_then(guid)?)))
        .collect();
    t.sort();
    t.into_iter().map(|e| e.1).collect()
}

/// Convert only unconstrained tracks, whose min and max sizing functions agree.
fn grid_track_sizing(size: Option<&Kv<'_>>) -> Option<Value> {
    let size = size?;
    let min = size.get("minSizing")?;
    let max = size.get("maxSizing")?;
    let min_type = min.get("type").and_then(Kv::str)?;
    let max_type = max.get("type").and_then(Kv::str)?;
    if min_type != max_type {
        return None;
    }
    match min_type {
        "FIXED" => {
            let px = num(min, "value")?;
            let max_px = num(max, "value")?;
            (px == max_px && px.is_finite() && (0.0..=MAX_GRID_TRACK_VALUE).contains(&px)).then(|| json!({"sizing": "fixed", "value": px}))
        }
        "FLEX" => {
            let fr = num(min, "value")?;
            let max_fr = num(max, "value")?;
            (fr == max_fr && fr.is_finite() && fr > 0.0 && fr <= MAX_GRID_TRACK_VALUE).then(|| json!({"sizing": "fill", "value": fr}))
        }
        "HUG" => Some(json!({"sizing": "hug", "value": 0.0})),
        _ => None,
    }
}

pub(super) fn layout_grid(g: &Kv) -> Value {
    let pattern = match (g.get("pattern").and_then(Kv::str), g.get("axis").and_then(Kv::str)) {
        (Some("GRID"), _) => "GRID",
        (_, Some("Y")) => "ROWS",
        _ => "COLUMNS",
    };
    let mut o = json!({"pattern": pattern, "alignment": g.get("type").and_then(Kv::str).unwrap_or("STRETCH")});
    for (from, to) in [("sectionSize", "sectionSize"), ("numSections", "count"), ("gutterSize", "gutterSize"), ("offset", "offset")] {
        if let Some(x) = num(g, from) {
            o[to] = json!(x);
        }
    }
    if let Some(c) = g.get("color") {
        o["color"] = color(c);
    }
    if let Some(b) = g.get("visible").and_then(Kv::bool) {
        o["visible"] = json!(b);
    }
    let vars = [("numSectionsVar", "count"), ("offsetVar", "offset"), ("sectionSizeVar", "sectionSize"), ("gutterSizeVar", "gutterSize")];
    if let Some(b) = bound(Some(g), &vars) {
        o["boundVariables"] = b;
    }
    o
}
