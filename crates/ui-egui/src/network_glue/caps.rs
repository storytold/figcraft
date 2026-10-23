//! The mapping of what a printer published onto craft-ui's three-state `Caps`, and of a `Request` onto
//! craft-print's per-job options. Pure: no network, no egui.
//!
//! Per option: unknown (not published, or the details read failed), supported (a list or range was
//! published), unsupported (the printer answered and the option is not in it). Capability is static;
//! the printer's status never reaches here.

use crate::netprint::ipp::PrinterAttrs;
use crate::netprint::wsd::PrinterElements;
use crate::netprint::{ColourMode, JobHint, JobOptions, PrinterDetails, Resolution};
use craft_ui::capability::Capability;
use craft_ui::printing::network::{self as net, Caps, Request};

/// The settings both backends apply as job options (what their `PrinterDetails::honoured` lists).
const HONOURED: &[JobHint] =
    &[JobHint::Copies, JobHint::Duplex, JobHint::Colour, JobHint::Media, JobHint::Orientation, JobHint::Quality, JobHint::Resolution];

/// Token lists the protocol details give beyond `PrinterDetails` (which has no field for them).
/// `None`: the printer did not publish the list; `Some(vec![])`: it published an empty one.
#[derive(Default)]
pub(crate) struct Listed {
    pub sides: Option<Vec<String>>,
    pub orientations: Option<Vec<String>>,
    pub qualities: Option<Vec<String>>,
}

fn tokens(list: Option<Vec<String>>) -> Option<Capability<String>> {
    list.map(|l| Capability::from_option_list(Some(l.as_slice())))
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// `Caps` from the backend's details. `formats`: the media types as the printer spelled them.
pub(crate) fn caps_from_details(details: &PrinterDetails, formats: Vec<String>, listed: Listed) -> Caps {
    // With no list from the printer, a backend that ignores the hint is evidence it is useless
    // (unsupported); otherwise nothing is known.
    let hint_only = |hint: JobHint| {
        if details.honoured.as_ref().is_some_and(|h| !h.contains(&hint)) { Capability::Unsupported } else { Capability::Unknown }
    };
    let sides = tokens(listed.sides).unwrap_or_else(|| match details.two_sided {
        None => Capability::Unknown,
        // The field cannot say which edges: offer both.
        Some(true) => Capability::from_option_list(Some(strings(&["OneSided", "TwoSidedLongEdge", "TwoSidedShortEdge"]).as_slice())),
        Some(false) => Capability::Unsupported,
    });
    let color_modes = match details.colour {
        None => Capability::Unknown,
        Some(true) => Capability::from_option_list(Some(strings(&["Color", "Monochrome"]).as_slice())),
        // The model reads a list without Color as unsupported.
        Some(false) => Capability::from_option_list(Some(strings(&["Monochrome"]).as_slice())),
    };
    let resolutions: Vec<net::Resolution> = details.resolutions.iter().map(|r| net::Resolution::new(r.x_dpi, r.y_dpi)).collect();
    Caps {
        formats,
        sides,
        orientations: tokens(listed.orientations).unwrap_or_else(|| hint_only(JobHint::Orientation)),
        qualities: tokens(listed.qualities).unwrap_or_else(|| hint_only(JobHint::Quality)),
        color_modes,
        // Empty is "not published", which a Vec cannot tell from "published nothing": unknown.
        media_sizes: if details.paper.is_empty() { Capability::Unknown } else { Capability::from_option_list(Some(details.paper.as_slice())) },
        resolutions: if resolutions.is_empty() { Capability::Unknown } else { Capability::from_option_list(Some(resolutions.as_slice())) },
        copies: Capability::from_option_range(details.max_copies.map(|n| (1, n))),
    }
}

fn base(name: String, colour: Option<bool>) -> PrinterDetails {
    PrinterDetails { name, colour, honoured: Some(HONOURED.to_vec()), ..PrinterDetails::default() }
}

/// What `GetPrinterElements` returned, as the WS-Print backend turns it into details.
pub(crate) fn wsd(elements: &PrinterElements) -> Caps {
    let description = elements.description.clone().unwrap_or_default();
    let mut details = base(description.name, description.color_supported);
    let mut formats = Vec::new();
    let mut listed = Listed::default();
    if let Some(caps) = &elements.capabilities {
        formats.clone_from(&caps.formats);
        details.two_sided = if caps.sides.is_empty() { None } else { Some(caps.sides.iter().any(|s| s.starts_with("TwoSided"))) };
        details.paper.clone_from(&caps.media_sizes);
        details.max_copies = caps.copies.map(|(_, max)| max);
        details.resolutions = caps.resolutions.iter().map(|&(x, y)| Resolution::new(x, y)).collect();
        // WS-Print cannot say "an empty list": an empty one is "not stated".
        let some = |v: &Vec<String>| (!v.is_empty()).then(|| v.clone());
        listed = Listed { sides: some(&caps.sides), orientations: some(&caps.orientations), qualities: some(&caps.qualities) };
    }
    caps_from_details(&details, formats, listed)
}

/// The media names the dialog's sheet sizes map to.
pub(crate) const PAPER_MEDIA: &[&str] =
    &["na_letter_8.5x11in", "na_legal_8.5x14in", "na_ledger_11x17in", "iso_a3_297x420mm", "iso_a4_210x297mm", "iso_a5_148x210mm"];

fn side_token(s: &str) -> Option<&'static str> {
    Some(match s {
        "one-sided" => "OneSided",
        "two-sided-long-edge" => "TwoSidedLongEdge",
        "two-sided-short-edge" => "TwoSidedShortEdge",
        _ => return None,
    })
}

fn orientation_token(o: i32) -> Option<&'static str> {
    Some(match o {
        3 => "Portrait",
        4 => "Landscape",
        5 => "ReverseLandscape",
        6 => "ReversePortrait",
        _ => return None,
    })
}

fn quality_token(q: i32) -> Option<&'static str> {
    Some(match q {
        3 => "Draft",
        4 => "Normal",
        5 => "High",
        _ => return None,
    })
}

/// What an IPP printer's attributes say, with the keywords as the dialog's tokens. A list the printer
/// published that holds no value the dialog knows is an empty list: unsupported.
pub(crate) fn ipp(attrs: &PrinterAttrs) -> Caps {
    let name = attrs.name.clone().or_else(|| attrs.info.clone()).or_else(|| attrs.make_and_model.clone()).unwrap_or_default();
    // Colour as the IPP backend reads it: a listed "color" mode, else the flag, else "only other modes".
    let colour = match (&attrs.color_modes, attrs.color_supported) {
        (Some(modes), _) if modes.iter().any(|m| m == "color") => Some(true),
        (_, Some(supported)) => Some(supported),
        (Some(modes), None) if !modes.is_empty() => Some(false),
        _ => None,
    };
    let mut details = base(name, colour);
    details.two_sided = attrs.sides.as_ref().map(|s| s.iter().any(|k| k.starts_with("two-sided")));
    details.paper = attrs.media.iter().flatten().filter(|m| PAPER_MEDIA.contains(&m.as_str())).cloned().collect();
    details.max_copies = attrs.copies.and_then(|(_, hi)| u32::try_from(hi).ok());
    details.resolutions = attrs.resolutions.iter().flatten().map(|&(x, y)| Resolution::new(x, y)).collect();
    let listed = Listed {
        sides: attrs.sides.as_ref().map(|l| l.iter().filter_map(|s| side_token(s)).map(str::to_owned).collect()),
        orientations: attrs.orientations.as_ref().map(|l| l.iter().filter_map(|o| orientation_token(*o)).map(str::to_owned).collect()),
        qualities: attrs.qualities.as_ref().map(|l| l.iter().filter_map(|q| quality_token(*q)).map(str::to_owned).collect()),
    };
    caps_from_details(&details, attrs.formats.clone(), listed)
}

/// The job options for what the request carries. An option the request omits (unsupported) is not
/// set; an unknown option the person chose is set and reaches the printer as a hint.
pub(crate) fn apply_request(mut options: JobOptions, request: &Request) -> JobOptions {
    if let Some(color) = request.color {
        options = options.with_colour(match color {
            net::ColorMode::Color => ColourMode::Colour,
            net::ColorMode::Monochrome => ColourMode::Mono,
        });
    }
    if let Some(r) = request.resolution {
        options = options.with_resolution(Resolution::new(r.x_dpi, r.y_dpi));
    }
    options
}

#[cfg(test)]
mod tests;
