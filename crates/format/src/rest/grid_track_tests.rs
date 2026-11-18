//! REST import of grid track sizes (`gridColumnsSizing`, `gridRowsSizing`: CSS
//! `grid-template-columns`/`-rows` strings). Fixtures are synthetic, written by hand.

use super::*;
use figcraft_doc::GridTrack;
use serde_json::json;

fn tracks(columns: Value, rows: Value) -> (Vec<GridTrack>, Vec<GridTrack>) {
    let file = json!({"name": "Fixture", "document": {"id": "0:0", "name": "Document", "type": "DOCUMENT", "children": [
        {"id": "0:1", "name": "Page", "type": "CANVAS", "children": [
            {"id": "1", "name": "Grid", "type": "FRAME", "relativeTransform": [[1, 0, 0], [0, 1, 0]], "size": {"x": 400, "y": 200},
             "layoutMode": "GRID", "gridColumnCount": 7, "gridRowCount": 2, "gridColumnsSizing": columns, "gridRowsSizing": rows}
        ]}
    ]}});
    let (doc, _) = import(&file, &no_images).unwrap();
    let layout = doc.nodes().find(|n| n.name == "Grid").unwrap().layout.clone().unwrap();
    (layout.column_tracks, layout.row_tracks)
}

#[test]
fn css_track_strings_set_the_track_sizes() {
    let (cols, rows) = tracks(json!("120px 2fr auto"), json!("repeat(2, minmax(0, 1fr))"));
    assert_eq!(cols, [GridTrack::fixed(120.0), GridTrack::fill(2.0), GridTrack::hug()]);
    assert_eq!(rows, [GridTrack::fill(1.0); 2]);
    let (cols, rows) = tracks(json!(" repeat(2, 40px 1.5fr)  minmax(64px, 64px) "), json!("min-content max-content"));
    assert_eq!(cols, [GridTrack::fixed(40.0), GridTrack::fill(1.5), GridTrack::fixed(40.0), GridTrack::fill(1.5), GridTrack::fixed(64.0)]);
    assert_eq!(rows, [GridTrack::hug(); 2]);
}

#[test]
fn unsupported_or_junk_tracks_are_equal_tracks() {
    // Ranges the grid can't express, units it doesn't have and junk keep their place as 1fr.
    let (cols, rows) = tracks(json!("minmax(10px, 80px) 5em -3px 0fr NaNpx repeat(x, 1px) 10px"), json!(7));
    assert_eq!(cols, [[GridTrack::default(); 6].as_slice(), &[GridTrack::fixed(10.0)]].concat());
    assert!(rows.is_empty(), "a non-string is ignored");
    // Tracks past the grid's own count are left out; huge repeats and deep nesting stay bounded.
    let (cols, rows) = tracks(json!(format!("repeat(999999999999, 1px) {}", "minmax(".repeat(10_000))), json!("1px 2px 3px"));
    assert_eq!(cols, [GridTrack::fixed(1.0); 7]);
    assert_eq!(rows, [GridTrack::fixed(1.0), GridTrack::fixed(2.0)]);
    let (cols, rows) = tracks(json!("1e9px"), json!("( ) )) repeat(2,)"));
    assert_eq!((cols, rows.len()), (vec![GridTrack::default()], 2));
}
