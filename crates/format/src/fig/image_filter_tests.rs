//! Image paint filters read from `.fig` files, on fixtures built in code with the parent module's
//! helpers.

use super::*;

/// The shared schema plus the image filter containers only these tests use.
fn image_filter_schema() -> Schema {
    let text = SCHEMA
        .replace(
            "message Paint ",
            "message FilterColorAdjust float tint, float shadows, float highlights, float detail, float exposure, float vignette, float temperature, float vibrance\n\
             message PaintFilterMessage float tint, float shadows, float highlights, float detail, float exposure, float vignette, float temperature, float vibrance, float contrast, float brightness, float mystery\n\
             message Paint ",
        )
        .replace("message Paint PaintType type,", "message Paint PaintFilterMessage paintFilter, FilterColorAdjust filterColorAdjust, PaintType type,");
    Schema::parse(&text).unwrap()
}

/// A file with one image rectangle per filter case (current container, legacy, both, neutral, unknown fields).
fn image_filter_fixture() -> Vec<u8> {
    let filter = |fields: Vec<(&'static str, Kv<'static>)>| o(fields);
    let image_paint = |id: u64, name: &'static str, mut extra: Vec<(&'static str, Kv<'static>)>| {
        let mut paint = vec![("type", e("IMAGE")), ("image", o(vec![("hash", Kv::Bytes(HASH.to_vec()))]))];
        paint.append(&mut extra);
        node(id, Some((1, "!")), "RECTANGLE", name, vec![("size", xy(2.0, 1.0)), ("fillPaints", arr(vec![o(paint)]))])
    };
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        image_paint(
            2,
            "Current",
            vec![(
                "paintFilter",
                filter(vec![
                    ("exposure", f(0.25)),
                    ("contrast", f(0.3)),
                    ("vibrance", f(-0.4)),
                    ("temperature", f(0.5)),
                    ("tint", f(1.0)),
                    ("highlights", f(-0.6)),
                    ("shadows", f(0.7)),
                ]),
            )],
        ),
        image_paint(3, "Negative tint", vec![("paintFilter", filter(vec![("tint", f(-1.0)), ("contrast", f(-0.3))]))]),
        image_paint(4, "Neutral", vec![]),
        image_paint(5, "Explicit neutral", vec![("paintFilter", filter(vec![("exposure", f(0.0)), ("vibrance", f(0.0))]))]),
        image_paint(6, "Legacy", vec![("filterColorAdjust", filter(vec![("exposure", f(0.8))]))]),
        image_paint(7, "Both", vec![("filterColorAdjust", filter(vec![("exposure", f(0.8))])), ("paintFilter", filter(vec![("contrast", f(0.2))]))]),
        image_paint(
            8,
            "Unsupported",
            vec![("paintFilter", filter(vec![("brightness", f(0.4)), ("detail", f(-0.2)), ("vignette", f(0.3)), ("mystery", f(0.1))]))],
        ),
    ];
    zip_file(&canvas(&image_filter_schema(), &o(vec![("nodeChanges", arr(nodes))])))
}

#[test]
fn image_filters_import_from_current_container_and_skip_ambiguous_data() {
    let bytes = image_filter_fixture();
    let (doc, warnings) = import(&bytes, "ImageFilters.fig").unwrap();
    let get = |name: &str| {
        let node = doc.nodes().find(|node| node.name == name).unwrap();
        let figcraft_doc::color::PaintKind::Image(image) = &node.fills[0].kind else { panic!("{name} is not an image fill") };
        image
    };
    let current = get("Current");
    assert!((current.filters.exposure - 0.25).abs() < 1e-6);
    assert!((current.filters.contrast - 0.3).abs() < 1e-6, "observed raw contrast remains 0.3");
    assert!((current.filters.saturation + 0.4).abs() < 1e-6, "vibrance maps to saturation");
    assert!((current.filters.temperature - 0.5).abs() < 1e-6);
    assert!((current.filters.tint + 1.0).abs() < 1e-6, "a positive (magenta) tint in the file is a negative native tint");
    assert!((current.filters.highlights + 0.6).abs() < 1e-6);
    assert!((current.filters.shadows - 0.7).abs() < 1e-6);
    assert!((get("Negative tint").filters.tint - 1.0).abs() < 1e-6);
    assert!((get("Negative tint").filters.contrast + 0.3).abs() < 1e-6);
    assert_eq!(get("Neutral").filters, Default::default());
    assert_eq!(get("Explicit neutral").filters, Default::default());
    assert_eq!(doc.images.get(&current.image).map(|image| image.bytes.as_slice()), Some(png().as_slice()));
    assert!(warnings.iter().any(|w| w.contains("legacy image filter container is unsupported")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("both filter containers are present")), "{warnings:?}");
    for field in ["brightness", "detail", "vignette", "mystery"] {
        assert!(warnings.iter().any(|w| w.contains(field)), "missing {field} warning: {warnings:?}");
    }
    assert_eq!(warnings.iter().filter(|w| w.contains("approximate renderer")).count(), 1, "deduplicated approximation warning: {warnings:?}");
}

#[test]
fn warnings_naming_unknown_filter_fields_are_capped() {
    // A schema can name any number of fields; each nonzero unknown one is a warning of its own.
    let names: Vec<&'static str> = (0..300).map(|i| &*Box::leak(format!("unknown{i}").into_boxed_str())).collect();
    let fields: Vec<String> = names.iter().map(|n| format!("float {n}")).collect();
    let text = SCHEMA
        .replace("message Paint ", &format!("message ManyFilters {}\nmessage Paint ", fields.join(", ")))
        .replace("message Paint PaintType type,", "message Paint ManyFilters paintFilter, PaintType type,");
    let filters = o(names.iter().map(|n| (*n, f(0.5))).collect());
    let paint = o(vec![("type", e("IMAGE")), ("image", o(vec![("hash", Kv::Bytes(HASH.to_vec()))])), ("paintFilter", filters)]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        node(2, Some((1, "!")), "RECTANGLE", "Photo", vec![("size", xy(2.0, 1.0)), ("fillPaints", arr(vec![paint]))]),
    ];
    let bytes = zip_file(&canvas(&Schema::parse(&text).unwrap(), &o(vec![("nodeChanges", arr(nodes))])));
    let (_, warnings) = import(&bytes, "ManyFilters.fig").unwrap();
    let named = warnings.iter().filter(|w| w.contains("unsupported image filter")).count();
    assert!((100..=crate::rest::MAX_WARNINGS).contains(&named), "{named} of {} warnings", warnings.len());
}

#[test]
fn image_adjustments_survive_a_fig_export() {
    let (doc, _) = import(&image_filter_fixture(), "ImageFilters.fig").unwrap();
    let filters_of = |doc: &figcraft_doc::Document, name: &str| {
        let node = doc.nodes().find(|node| node.name == name).unwrap();
        let figcraft_doc::color::PaintKind::Image(image) = &node.fills[0].kind else { panic!("{name} is not an image fill") };
        image.filters
    };
    let (bytes, warnings) = export::export(&doc, None).unwrap();
    assert!(!warnings.iter().any(|w| w.contains("image adjustments")), "{warnings:?}");
    let (back, _) = import(&bytes, "Back.fig").unwrap();
    for name in ["Current", "Negative tint", "Neutral"] {
        let (a, b) = (filters_of(&doc, name), filters_of(&back, name));
        for (x, y) in [
            (a.exposure, b.exposure),
            (a.contrast, b.contrast),
            (a.saturation, b.saturation),
            (a.temperature, b.temperature),
            (a.tint, b.tint),
            (a.highlights, b.highlights),
            (a.shadows, b.shadows),
        ] {
            assert!((x - y).abs() < 1e-6, "{name}: {a:?} came back as {b:?}");
        }
    }
    assert!(filters_of(&back, "Neutral").is_identity());
    assert!(!filters_of(&back, "Current").is_identity());

    // What is written: the file's names and signs.
    let archive = Archive::open(&bytes).unwrap();
    let (schema, message) = decode_canvas(&archive.canvas).unwrap();
    let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    let fill_of = |name: &str| {
        let nc = msg.get("nodeChanges").unwrap().list().iter().find(|nc| nc.get("name").and_then(Kv::str) == Some(name)).unwrap();
        nc.get("fillPaints").unwrap().list()[0].clone()
    };
    let current = fill_of("Current");
    let written = |paint: &Kv, field: &str| paint.get("paintFilter").and_then(|f| f.get(field)).and_then(Kv::f64);
    assert!((written(&current, "vibrance").unwrap() + 0.4).abs() < 1e-6);
    assert!((written(&current, "tint").unwrap() - 1.0).abs() < 1e-6);
    assert!((written(&current, "shadows").unwrap() - 0.7).abs() < 1e-6);
    assert!(fill_of("Neutral").get("paintFilter").is_none());

    // The numbers the format uses.
    let s = Schema::parse(export::SCHEMA).unwrap();
    let def = |n: &str| s.defs.iter().find(|d| d.name == n).unwrap();
    let id = |d: &str, f: &str| def(d).fields.iter().find(|x| x.name == f).unwrap().value;
    assert_eq!(id("Paint", "paintFilter"), 14);
    assert_eq!(
        ["tint", "shadows", "highlights", "exposure", "temperature", "vibrance", "contrast"].map(|f| id("PaintFilterMessage", f)),
        [1, 2, 3, 5, 7, 8, 9]
    );
}
