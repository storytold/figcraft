use super::*;
use craft_ui::capability::{Advertised, Choices};
use craft_ui::printing::network::{Draft, Failure, FailureKind, PrinterSummary};

fn details() -> PrinterDetails {
    PrinterDetails::default()
}

fn list<T: Clone>(c: &Capability<T>) -> Vec<T> {
    c.list().to_vec()
}

/// A draft that has been told the printer answered with `caps`.
fn answered(caps: Caps) -> Draft {
    let mut draft = Draft::for_address("127.0.0.1");
    let generation = draft.begin_query();
    let url = draft.normalized_url();
    draft.finish_query(generation, &url, Ok(PrinterSummary { caps, ..PrinterSummary::default() }));
    draft
}

#[test]
fn nothing_published_is_unknown_everywhere() {
    let caps = caps_from_details(&details(), vec![], Listed::default());
    assert_eq!(caps, Caps::default());
}

#[test]
fn two_sided_none_some_true_some_false() {
    let sides = |two_sided| caps_from_details(&PrinterDetails { two_sided, ..details() }, vec![], Listed::default()).sides;
    assert_eq!(sides(None), Capability::Unknown);
    assert_eq!(list(&sides(Some(true))), ["OneSided", "TwoSidedLongEdge", "TwoSidedShortEdge"]);
    assert_eq!(sides(Some(false)), Capability::Unsupported);
}

#[test]
fn colour_none_some_true_some_false() {
    let colour = |colour| caps_from_details(&PrinterDetails { colour, ..details() }, vec![], Listed::default());
    assert_eq!(colour(None).color_modes, Capability::Unknown);
    assert_eq!(list(&colour(Some(true)).color_modes), ["Color", "Monochrome"]);
    let mono = colour(Some(false));
    assert_eq!(list(&mono.color_modes), ["Monochrome"]);
    // The model reads a list without Color as unsupported.
    assert_eq!(answered(mono).color_state(), Capability::Unsupported);
}

#[test]
fn copies_are_a_range_from_the_maximum() {
    let copies = |max_copies| caps_from_details(&PrinterDetails { max_copies, ..details() }, vec![], Listed::default()).copies;
    assert_eq!(copies(None), Capability::Unknown);
    assert_eq!(copies(Some(10)), Capability::Supported(Advertised { choices: Choices::Range { min: 1, max: 10 }, default: None }));
    assert!(!copies(Some(0)).is_enabled(), "a maximum of zero is not a choice");
}

#[test]
fn empty_lists_are_unknown_and_listed_ones_supported() {
    let empty = caps_from_details(&details(), vec![], Listed::default());
    assert_eq!((empty.media_sizes, empty.resolutions), (Capability::Unknown, Capability::Unknown));
    let listed = caps_from_details(
        &PrinterDetails {
            paper: vec!["iso_a4_210x297mm".into()],
            resolutions: vec![Resolution::new(300, 300), Resolution::new(600, 300)],
            ..details()
        },
        vec!["application/pdf".into()],
        Listed::default(),
    );
    assert_eq!(list(&listed.media_sizes), ["iso_a4_210x297mm"]);
    assert_eq!(list(&listed.resolutions), [net::Resolution::new(300, 300), net::Resolution::new(600, 300)]);
    assert_eq!(listed.formats, ["application/pdf"]);
}

#[test]
fn orientation_and_quality_follow_honoured_unless_the_printer_lists_them() {
    let none = caps_from_details(&PrinterDetails { honoured: None, ..details() }, vec![], Listed::default());
    assert_eq!((none.orientations, none.qualities), (Capability::Unknown, Capability::Unknown));
    let applied = caps_from_details(&PrinterDetails { honoured: Some(HONOURED.to_vec()), ..details() }, vec![], Listed::default());
    assert_eq!((applied.orientations, applied.qualities), (Capability::Unknown, Capability::Unknown));
    let ignored = caps_from_details(&PrinterDetails { honoured: Some(vec![JobHint::Copies]), ..details() }, vec![], Listed::default());
    assert_eq!((ignored.orientations, ignored.qualities), (Capability::Unsupported, Capability::Unsupported));
    let listed = Listed { orientations: Some(vec!["Portrait".into()]), qualities: Some(vec![]), ..Listed::default() };
    let from_printer = caps_from_details(&PrinterDetails { honoured: Some(vec![]), ..details() }, vec![], listed);
    assert_eq!(list(&from_printer.orientations), ["Portrait"]);
    assert_eq!(from_printer.qualities, Capability::Unsupported, "a published empty list");
}

#[test]
fn a_failed_read_leaves_every_option_unknown() {
    let caps = caps_from_details(
        &PrinterDetails { two_sided: Some(false), colour: Some(false), max_copies: Some(1), ..details() },
        vec![],
        Listed::default(),
    );
    let mut draft = answered(caps);
    assert_eq!(draft.sides_state(), Capability::Unsupported);
    assert_eq!(draft.copies_state(), Capability::Unsupported);
    let generation = draft.begin_query();
    let url = draft.normalized_url();
    draft.finish_query(generation, &url, Err(Failure::new(FailureKind::Transport, "unreachable")));
    assert_eq!(draft.sides_state(), Capability::Unknown);
    assert_eq!(draft.color_state(), Capability::Unknown);
    assert_eq!(draft.copies_state(), Capability::Unknown);
    assert_eq!(draft.resolution_state(), Capability::Unknown);
}

#[test]
fn the_request_omits_what_the_printer_does_not_support_and_maps_what_it_does() {
    let caps = caps_from_details(
        &PrinterDetails { colour: Some(false), resolutions: vec![Resolution::new(300, 300)], ..details() },
        vec![],
        Listed::default(),
    );
    let mut draft = answered(caps);
    draft.color = Some(net::ColorMode::Color);
    draft.resolution = Some(net::Resolution::new(300, 300));
    let request = draft.request(None);
    assert_eq!(request.color, None, "unsupported: not sent");
    assert_eq!(request.resolution, Some(net::Resolution::new(300, 300)));
    let options = apply_request(JobOptions::new(), &request);
    assert_eq!((options.colour, options.resolution), (ColourMode::Auto, Some(Resolution::new(300, 300))));

    let mut request = Request { color: Some(net::ColorMode::Monochrome), resolution: Some(net::Resolution::new(600, 300)), ..Request::default() };
    let options = apply_request(JobOptions::new(), &request);
    assert_eq!((options.colour, options.resolution), (ColourMode::Mono, Some(Resolution::new(600, 300))));
    request.color = Some(net::ColorMode::Color);
    assert_eq!(apply_request(JobOptions::new(), &request).colour, ColourMode::Colour);
    let none = apply_request(JobOptions::new(), &Request::default());
    assert_eq!((none.colour, none.resolution), (ColourMode::Auto, None));
}

#[test]
fn ipp_attributes_map_to_the_three_states() {
    let attrs = PrinterAttrs {
        sides: Some(vec!["one-sided".into()]),
        color_supported: Some(true),
        copies: Some((1, 5)),
        qualities: Some(vec![4]),
        ..PrinterAttrs::default()
    };
    let caps = ipp(&attrs);
    assert_eq!(list(&caps.sides), ["OneSided"], "the model reads this as no two-sided");
    assert_eq!(list(&caps.color_modes), ["Color", "Monochrome"]);
    assert_eq!(caps.orientations, Capability::Unknown);
    assert_eq!(list(&caps.qualities), ["Normal"]);
    assert_eq!(caps.resolutions, Capability::Unknown);
    assert_eq!(caps.media_sizes, Capability::Unknown);
    let nothing = ipp(&PrinterAttrs::default());
    assert_eq!((nothing.sides, nothing.copies, nothing.color_modes), (Capability::Unknown, Capability::Unknown, Capability::Unknown));
}
