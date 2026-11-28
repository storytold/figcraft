use super::*;
use figcraft_color::{Color, Paint};
use figcraft_geom::Affine;

const RED: Color = Color::from_rgb8_const(255, 0, 0);
const GREEN: Color = Color::from_rgb8_const(0, 255, 0);
const BLUE: Color = Color::from_rgb8_const(0, 0, 255);

fn scene() -> (Session, NodeId, NodeId) {
    let mut doc = Document::new("Thumbnail");
    let page = doc.first_page().unwrap();
    let cover = doc.create(page, NodeKind::Frame, "Cover", 0.0, 600.0, 100.0, 100.0).unwrap();
    let wide = doc.create(page, NodeKind::Frame, "Wide", 700.0, 600.0, 600.0, 100.0).unwrap();
    doc.get_mut(cover).unwrap().fills = vec![Paint::solid(RED)];
    doc.get_mut(wide).unwrap().fills = vec![Paint::solid(BLUE)];
    (Session::with_document(doc), cover, wide)
}

fn choose(s: &mut Session, id: NodeId) {
    s.execute("object.setThumbnail", &json!({"id": id})).unwrap();
}

fn picture(doc: &Document) -> image::RgbaImage {
    image::load_from_memory(&thumbnail(doc).unwrap()).unwrap().to_rgba8()
}

#[test]
fn chosen_frame_draws_its_children_without_siblings_or_document_changes() {
    let (mut s, cover, _) = scene();
    let doc = s.doc_mut().unwrap();
    let page = doc.first_page().unwrap();
    let child = doc.create(cover, NodeKind::Rectangle, "Child", 20.0, 20.0, 20.0, 20.0).unwrap();
    doc.get_mut(child).unwrap().fills = vec![Paint::solid(GREEN)];
    // Cropping the whole page to Cover's bounds would still draw this overlapping sibling.
    let sibling = doc.create(page, NodeKind::Rectangle, "Overlap", 0.0, 600.0, 100.0, 100.0).unwrap();
    doc.get_mut(sibling).unwrap().fills = vec![Paint::solid(BLUE)];
    choose(&mut s, cover);
    let before = s.doc().unwrap().clone();
    let png = picture(s.doc().unwrap());
    assert_eq!(png.dimensions(), (200, 200));
    assert_eq!(png.get_pixel(10, 10).0, [255, 0, 0, 255]);
    assert_eq!(png.get_pixel(60, 60).0, [0, 255, 0, 255]);
    assert!(!png.pixels().any(|p| p.0 == [0, 0, 255, 255]), "neither blue sibling is drawn");
    assert_eq!(s.doc().unwrap(), &before);
    s.doc_mut().unwrap().set_world_transform(cover, Affine::translate((4200.0, -3600.0))).unwrap();
    assert_eq!(picture(s.doc().unwrap()), png, "moving the frame changes neither size nor contents");
}

#[test]
fn no_thumbnail_choice_keeps_the_first_page_content_bounds() {
    let (s, _, _) = scene();
    let png = picture(s.doc().unwrap());
    assert_eq!(png.dimensions(), (400, 31));
    assert_eq!(png.get_pixel(10, 15).0, [255, 0, 0, 255]);
    assert_eq!(png.get_pixel(310, 15).0, [0, 0, 255, 255]);
}

#[test]
fn invalid_flags_and_non_frames_keep_the_default_thumbnail() {
    let (mut s, cover, wide) = scene();
    let default = thumbnail(s.doc().unwrap()).unwrap();
    for value in ["false", "TRUE", ""] {
        s.doc_mut().unwrap().get_mut(cover).unwrap().plugin_data.insert("figcraft.thumbnail".into(), value.into());
        assert_eq!(thumbnail(s.doc().unwrap()).unwrap(), default, "flag {value:?}");
    }
    choose(&mut s, wide);
    s.doc_mut().unwrap().get_mut(wide).unwrap().kind = NodeKind::Rectangle;
    assert_eq!(thumbnail(s.doc().unwrap()).unwrap(), default, "only frames can be thumbnails");
    s.doc_mut().unwrap().get_mut(wide).unwrap().kind =
        NodeKind::Component { description: String::new(), props: Default::default(), prop_order: Vec::new() };
    assert_ne!(thumbnail(s.doc().unwrap()).unwrap(), default, "a component counts as a frame");
}

#[test]
fn deleting_the_chosen_frame_falls_back_to_the_remaining_page_content() {
    let (mut s, cover, _) = scene();
    choose(&mut s, cover);
    s.doc_mut().unwrap().remove(cover).unwrap();
    let png = picture(s.doc().unwrap());
    assert_eq!(png.dimensions(), (400, 67));
    assert_eq!(png.get_pixel(200, 30).0, [0, 0, 255, 255]);
}

#[test]
fn invalid_thumbnail_ranges_and_pages_fall_back() {
    let (mut s, _, _) = scene();
    let default = thumbnail(s.doc().unwrap()).unwrap();
    let doc = s.doc_mut().unwrap();
    let page = doc.add_page("Invalid cover");
    let cover = doc.create(page, NodeKind::Frame, "Cover", 0.0, 0.0, 100.0, 100.0).unwrap();
    choose(&mut s, cover);
    for (width, height) in [(0.0, 100.0), (100.0, 0.0), (f64::INFINITY, 100.0), (f64::NAN, 100.0)] {
        let n = s.doc_mut().unwrap().get_mut(cover).unwrap();
        n.width = width;
        n.height = height;
        assert_eq!(thumbnail(s.doc().unwrap()).unwrap(), default, "invalid range {width} x {height}");
    }
    let doc = s.doc_mut().unwrap();
    let n = doc.get_mut(cover).unwrap();
    n.width = 100.0;
    n.height = 100.0;
    n.parent = Some(NodeId(u64::MAX));
    assert_eq!(thumbnail(doc).unwrap(), default, "missing parent page");
    doc.get_mut(cover).unwrap().parent = Some(page);
    doc.pages.retain(|p| *p != page);
    assert_eq!(thumbnail(doc).unwrap(), default, "page outside the document's page list");
}

#[test]
fn a_nested_frame_on_another_page_uses_its_page_background() {
    let (mut s, _, _) = scene();
    let doc = s.doc_mut().unwrap();
    let page = doc.add_page("Cover page");
    if let NodeKind::Page { background, .. } = &mut doc.get_mut(page).unwrap().kind {
        *background = GREEN;
    }
    let section = doc.create(page, NodeKind::Section { contents_hidden: false }, "Section", 700.0, 900.0, 300.0, 200.0).unwrap();
    let cover = doc.create(section, NodeKind::Frame, "Cover", -50.0, 80.0, 80.0, 40.0).unwrap();
    doc.get_mut(cover).unwrap().fills.clear();
    let child = doc.create(cover, NodeKind::Rectangle, "Child", 10.0, 10.0, 20.0, 10.0).unwrap();
    doc.get_mut(child).unwrap().fills = vec![Paint::solid(RED)];
    choose(&mut s, cover);
    let png = picture(s.doc().unwrap());
    assert_eq!(png.dimensions(), (160, 80));
    assert_eq!(png.get_pixel(10, 10).0, [0, 255, 0, 255]);
    assert_eq!(png.get_pixel(30, 30).0, [255, 0, 0, 255]);
}

#[test]
fn custom_thumbnails_keep_the_size_limit_and_aspect_ratio() {
    let (mut s, cover, _) = scene();
    choose(&mut s, cover);
    for (width, height, want) in [(100.0, 100.0, (200, 200)), (800.0, 200.0, (400, 100)), (200.0, 800.0, (100, 400))] {
        let n = s.doc_mut().unwrap().get_mut(cover).unwrap();
        n.width = width;
        n.height = height;
        assert_eq!(picture(s.doc().unwrap()).dimensions(), want);
    }
}

#[test]
fn native_save_and_fig_export_embed_the_same_chosen_thumbnail() {
    let (mut s, cover, _) = scene();
    choose(&mut s, cover);
    let before = s.doc().unwrap().clone();
    let expected = picture(s.doc().unwrap());
    let native = file_bytes(s.ds().unwrap()).unwrap();
    let (fig, _) = super::export::fig_bytes(s.doc().unwrap()).unwrap();
    for bytes in [native, fig] {
        let png = figcraft_format::thumbnail::thumbnail_png(std::io::Cursor::new(bytes)).unwrap();
        let actual = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(actual.dimensions(), (200, 200));
        assert_eq!(actual, expected);
    }
    assert_eq!(s.doc().unwrap(), &before);
}
