//! Text box sizes measured against documents re-saved by other software. Local only: runs when
//! `FIGCRAFT_FIG_SAMPLES` names a folder of `.fig` files, and only for text whose exact font is
//! installed. The rules themselves are tested on built-in fonts in `figcraft_text`.

use figcraft_doc::{LineHeight, Node, NodeKind, TextAutoResize};

/// `n` with its percent line height read as a share of the font's own line height (the import
/// reads every percent as a share of the font size; files can mean either).
fn of_own_line_height(n: &Node) -> Option<Node> {
    let mut m = n.clone();
    let NodeKind::Text(t) = &mut m.kind else { return None };
    let LineHeight::Percent(p) = t.style.line_height else { return None };
    let f = figcraft_text::find_face(&t.style.font_family, &t.style.font_style)?;
    t.style.line_height = LineHeight::Percent(p * (f.ascent + f.descent + f.line_gap.max(0.0)) / f.upem);
    Some(m)
}

/// The chars that start a wrapped line (not those after a hard break) in `t` laid out `w` wide.
fn soft_starts(t: &figcraft_doc::TextData, w: f64) -> Vec<usize> {
    let chars: Vec<char> = t.characters.chars().collect();
    let l = figcraft_text::layout(t, Some(w), None);
    l.lines
        .iter()
        .skip(1)
        .map(|ln| ln.char_start)
        .filter(|s| !s.checked_sub(1).and_then(|i| chars.get(i)).is_some_and(|c| matches!(c, '\n' | '\u{2028}')))
        .collect()
}

/// Each fixed-width text with the line breaks its file saved (one-line texts too) wraps there once imported, unless a
/// line would overflow its box by more than the allowed slack in the fonts at hand (then it wraps
/// as it would without them). Prints the counts (`--nocapture`).
#[test]
fn local_samples_wrap_at_their_saved_line_breaks() {
    let Some(dir) = std::env::var_os("FIGCRAFT_FIG_SAMPLES") else { return };
    let Ok(dir) = std::fs::read_dir(dir) else { return };
    figcraft_text::scan_system_fonts();
    let (mut matched, mut fell_back, mut differed_before, mut wrapped) = (0, 0, 0, 0);
    let mut problems = Vec::new();
    for p in dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "fig")) {
        let Ok((doc, _)) = figcraft_format::fig::import(&std::fs::read(&p).unwrap(), "sample") else { continue };
        for n in doc.nodes() {
            let NodeKind::Text(t) = &n.kind else { continue };
            let Some(saved) = t.saved_breaks.as_deref() else { continue };
            if t.auto_resize == TextAutoResize::WidthAndHeight {
                continue;
            }
            wrapped += usize::from(!saved.starts.is_empty());
            let plain = figcraft_doc::TextData { saved_breaks: None, ..t.clone() };
            let (own, hinted) = (soft_starts(&plain, n.width), soft_starts(t, n.width));
            differed_before += usize::from(own != saved.starts);
            if t.saved_breaks_for(n.width).is_none() {
                problems.push(format!("{}: {:?}: saved breaks no longer apply after import", p.display(), t.characters));
            } else if hinted == saved.starts {
                matched += 1;
            } else if hinted == own {
                fell_back += 1;
            } else {
                problems.push(format!("{}: {:?}: {hinted:?} != saved {:?}", p.display(), t.characters, saved.starts));
            }
        }
    }
    eprintln!(
        "saved line breaks ({wrapped} of the texts wrap): {matched} matched, {fell_back} fell back, {} other; {differed_before} wrapped differently without them",
        problems.len()
    );
    assert!(problems.is_empty(), "{problems:#?}");
}

/// Each text box that sizes itself comes out at the size stored in the file, to the pixel.
#[test]
fn local_samples_keep_their_text_box_sizes() {
    let Some(dir) = std::env::var_os("FIGCRAFT_FIG_SAMPLES") else { return };
    let Ok(dir) = std::fs::read_dir(dir) else { return };
    figcraft_text::scan_system_fonts();
    for p in dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "fig")) {
        let Ok((doc, _)) = figcraft_format::fig::import(&std::fs::read(&p).unwrap(), "sample") else { continue };
        for n in doc.nodes() {
            let NodeKind::Text(t) = &n.kind else { continue };
            // Stored sizes are whole pixels once the text was laid out by the software that saved it.
            let laid_out = n.height.fract() == 0.0 && (t.auto_resize != TextAutoResize::WidthAndHeight || n.width.fract() == 0.0);
            let own_font = t.spans().iter().all(|(_, _, s)| {
                figcraft_text::find_face(&s.font_family, &s.font_style).is_some_and(|f| f.family == s.font_family && f.style == s.font_style)
            });
            let Some((w, h)) = figcraft_text::auto_size(n).filter(|_| laid_out && own_font) else { continue };
            let h = if h == n.height { h } else { of_own_line_height(n).and_then(|m| figcraft_text::auto_size(&m)).map_or(h, |s| s.1) };
            assert_eq!((w, h), (n.width, n.height), "{}: {:?}", p.display(), t.characters);
        }
    }
}
