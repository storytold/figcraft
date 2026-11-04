//! The sessions you joined, on the home screen's Online section (#1882): a grid of cards like the
//! recent files', each with a preview from the last time the session was open here and whether its
//! server can be reached. A card whose server can't be reached shows its preview grey, and says
//! whether the server is down or this computer is offline. A click joins again.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::Document;
use figcraft_engine::joined::{JoinedFile, preview_key};
use figcraft_engine::live::{Reach, Role, Status};
use figcraft_format::thumbnail::decode_thumbnail;
use serde_json::json;

use super::home_recents::{FOOTER_H, GAP, PICTURE_PX, ago, card_heights, grid_columns};
use crate::FigApp;
use crate::a11y;
use crate::i18n::{tr, trf};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

const JOINED: &str = "collab.joinedFiles";
/// How often the servers are asked while the section shows, and how often an open session's preview
/// is taken again when its file changed.
const PROBE_SECS: f64 = 30.0;
const CAPTURE_SECS: f64 = 60.0;

/// A preview in its colours, and grey for a server that can't be reached.
struct Picture {
    color: egui::TextureHandle,
    grey: egui::TextureHandle,
}

/// What the section keeps between frames.
#[derive(Default)]
pub struct JoinedUi {
    /// By preview key; `None`: there's no preview.
    pictures: HashMap<String, Option<Picture>>,
    probed: Option<f64>,
    /// By preview key: when its preview was last taken, and of which document.
    captured: HashMap<String, (f64, Weak<Document>)>,
    capturing: Arc<AtomicBool>,
    /// Sessions forgotten while their preview may be on its way: it isn't kept. Held across the
    /// move into place, so forgetting either waits for it and removes it, or stops it.
    forgotten: Arc<Mutex<HashSet<String>>>,
    /// Previews taken since the section last drew: their cards read them again.
    taken: Arc<Mutex<Vec<String>>>,
}

impl JoinedUi {
    /// The section shows after something else did: previews may be newer, and servers may have come
    /// and gone.
    pub(crate) fn shown_again(&mut self) {
        self.pictures.clear();
        self.probed = None;
    }
}

/// Remove the previews in `dir` of sessions no longer listed (the list keeps the latest
/// [`MAX_JOINED_FILES`](figcraft_engine::joined::MAX_JOINED_FILES): older ones have no card to forget
/// them from).
pub(crate) fn prune_previews(dir: &Path, files: &[JoinedFile]) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let listed = path.file_stem().is_some_and(|stem| files.iter().any(|f| f.preview_key() == stem.to_string_lossy()));
        if !listed && path.extension().is_some_and(|x| x == "png") {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The file of preview `key` in `dir`.
fn preview_file(dir: &Path, key: &str) -> std::path::PathBuf {
    dir.join(format!("{key}.png"))
}

/// The textures of the preview in `bytes`.
fn picture(ctx: &egui::Context, key: &str, bytes: &[u8]) -> Option<Picture> {
    let t = decode_thumbnail(bytes, PICTURE_PX)?;
    let size = [t.width as usize, t.height as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, &t.rgba);
    let mut grey_rgba = t.rgba;
    for px in grey_rgba.chunks_exact_mut(4) {
        let luma = ((u32::from(px[0]) * 54 + u32::from(px[1]) * 183 + u32::from(px[2]) * 19) >> 8) as u8;
        // Pale too: it reads as "as it was", not as a grey design.
        let faded = luma / 2 + 64;
        px[0] = faded;
        px[1] = faded;
        px[2] = faded;
    }
    let grey = egui::ColorImage::from_rgba_unmultiplied(size, &grey_rgba);
    Some(Picture {
        color: ctx.load_texture(format!("joined-{key}"), color, egui::TextureOptions::LINEAR),
        grey: ctx.load_texture(format!("joined-grey-{key}"), grey, egui::TextureOptions::LINEAR),
    })
}

/// Keep previews of the sessions you're in, for the home screen: when one has none yet, then at
/// most every [`CAPTURE_SECS`] while its file changes. They're drawn away from the window's thread,
/// one at a time.
pub(crate) fn capture(app: &mut FigApp, ctx: &egui::Context, time: f64) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let state = &mut app.ui.online.joined;
        if state.capturing.load(Ordering::Acquire) {
            return;
        }
        let Some(dir) = app.services.joined_previews_dir() else { return };
        for live in app.session.live.iter().filter(|l| l.status == Status::Live && l.syncing() && l.published.is_none()) {
            let Some(info) = live.info.as_ref().filter(|i| i.role == Role::Guest) else { continue };
            let Some(doc) = live.doc_uid.and_then(|u| app.session.docs.iter().find(|d| d.uid == u)).map(|d| d.doc.clone()) else { continue };
            let Some(file) = app.session.prefs.joined_files.iter().find(|f| f.link == info.link) else { continue };
            let key = preview_key(&file.address, &file.file);
            let due = state.captured.get(&key).is_none_or(|(at, last)| time >= at + CAPTURE_SECS && !Weak::ptr_eq(last, &Arc::downgrade(&doc)));
            if !due {
                continue;
            }
            state.captured.insert(key.clone(), (time, Arc::downgrade(&doc)));
            state.capturing.store(true, Ordering::Release);
            let busy = state.capturing.clone();
            let forgotten = state.forgotten.clone();
            let taken = state.taken.clone();
            let ctx = ctx.clone();
            forgotten.lock().unwrap_or_else(PoisonError::into_inner).remove(&key);
            let path = preview_file(&dir, &key);
            let spawned = std::thread::Builder::new().name("joined-preview".into()).spawn(move || {
                let png = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| figcraft_engine::cmd::file::thumbnail(&doc))).ok().flatten();
                if let Some(png) = png {
                    let tmp = path.with_extension("tmp");
                    let written = path.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(&tmp, png).is_ok();
                    let forgotten = forgotten.lock().unwrap_or_else(PoisonError::into_inner);
                    if !written || forgotten.contains(&key) {
                        let _ = std::fs::remove_file(&tmp);
                    } else if let Err(e) = std::fs::rename(&tmp, &path) {
                        log::warn!("couldn't keep a session's preview: {e}");
                    } else {
                        taken.lock().unwrap_or_else(PoisonError::into_inner).push(key.clone());
                        ctx.request_repaint();
                    }
                }
                busy.store(false, Ordering::Release);
            });
            if spawned.is_err() {
                state.capturing.store(false, Ordering::Release);
            }
            return;
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (app, ctx, time);
    }
}

/// What the card says about the server of `f`.
fn status(app: &FigApp, f: &JoinedFile) -> Option<Reach> {
    app.session.live.reach.get(&f.address).copied()
}

/// One card.
struct Card<'a> {
    file: &'a JoinedFile,
    reach: Option<Reach>,
    picture: Option<&'a Picture>,
    rect: Rect,
    picture_h: f32,
    now: u64,
}

fn paint(ui: &Ui, t: &Tokens, resp: &egui::Response, c: &Card) {
    let p = ui.painter();
    let (rect, file) = (c.rect, c.file);
    p.rect_filled(rect, CornerRadius::same(8), if resp.hovered() { t.hover } else { t.panel });
    let area = Rect::from_min_size(rect.min, vec2(rect.width(), c.picture_h)).shrink(1.0);
    p.rect_filled(area, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, t.canvas);
    let away = matches!(c.reach, Some(Reach::ServerDown | Reach::NoNetwork));
    match c.picture {
        Some(pic) => {
            let texture = if away { &pic.grey } else { &pic.color };
            let size = texture.size_vec2();
            let inner = area.shrink(area.height() * 0.05);
            let fit = (inner.width() / size.x.max(1.0)).min(inner.height() / size.y.max(1.0));
            p.image(
                texture.id(),
                Rect::from_center_size(inner.center(), size * fit),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => icons::paint(p, Rect::from_center_size(area.center(), vec2(28.0, 28.0)), Icon::LiveShared, t.icon_secondary),
    }
    // The server, in the picture's corner: a dot while it answers, words when it doesn't.
    let label = match c.reach {
        Some(Reach::Online) | None => None,
        Some(Reach::ServerDown) => Some((tr("Server not reachable"), t.danger)),
        Some(Reach::NoNetwork) => Some((tr("You're offline"), t.icon_secondary)),
    };
    let corner = pos2(area.max.x - 10.0, area.min.y + 10.0);
    if let Some((text, dot)) = label {
        let galley = p.layout_no_wrap(text.to_string(), regular(11.0), t.text);
        let pill = Rect::from_min_max(pos2(corner.x - galley.size().x - 26.0, corner.y), pos2(corner.x, corner.y + 22.0));
        p.rect_filled(pill, CornerRadius::same(11), t.panel);
        p.rect_stroke(pill, CornerRadius::same(11), Stroke::new(1.0, t.border), StrokeKind::Inside);
        p.circle_filled(pos2(pill.min.x + 12.0, pill.center().y), 3.5, dot);
        p.galley(pos2(pill.min.x + 20.0, pill.center().y - galley.size().y / 2.0), galley, t.text);
    } else if c.reach == Some(Reach::Online) {
        p.circle_filled(pos2(corner.x - 4.0, corner.y + 4.0), 4.0, t.success);
    }
    p.line_segment([pos2(rect.min.x, area.max.y + 0.5), pos2(rect.max.x, area.max.y + 0.5)], Stroke::new(1.0, t.border));
    let y = area.max.y + 1.0 + FOOTER_H / 2.0;
    icons::paint(
        p,
        Rect::from_center_size(pos2(rect.min.x + 24.0, y), vec2(16.0, 16.0)),
        Icon::LiveShared,
        if away { t.icon_secondary } else { t.success },
    );
    let (x, w) = (rect.min.x + 44.0, rect.width() - 56.0);
    p.text(pos2(x, y - 8.0), Align2::LEFT_CENTER, widgets::elide_font(ui, &file.name, w, semibold(12.5)), semibold(12.5), t.text);
    let when = trf("Opened {when}", &[("when", &ago(c.now, file.opened))]);
    let detail = format!("{} · {when}", file.address);
    p.text(pos2(x, y + 9.0), Align2::LEFT_CENTER, widgets::elide_font(ui, &detail, w, regular(11.5)), regular(11.5), t.text_secondary);
    p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, if resp.hovered() { t.border_strong } else { t.border }), StrokeKind::Inside);
    if resp.has_focus() {
        p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(2.0, t.focus), StrokeKind::Outside);
    }
}

/// The grid, when there's anything in it. Asks the servers from time to time.
pub(crate) fn section(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let files = app.session.prefs.joined_files.clone();
    if files.is_empty() {
        return;
    }
    let ctx = ui.ctx().clone();
    let time = ui.input(|i| i.time);
    let taken = std::mem::take(&mut *app.ui.online.joined.taken.lock().unwrap_or_else(PoisonError::into_inner));
    for key in taken {
        app.ui.online.joined.pictures.remove(&key);
    }
    if app.ui.online.joined.probed.is_none_or(|at| time >= at + PROBE_SECS) && !app.session.waiting_for(JOINED) {
        app.ui.online.joined.probed = Some(time);
        app.run_background(&ctx, JOINED, json!({"refresh": true}), Box::new(|_, _, _| {}));
    }
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(PROBE_SECS));
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tr("Sessions you joined")).font(semibold(13.0)).color(t.text));
    ui.add_space(10.0);
    let now = figcraft_engine::cmd::comment::now_secs();
    let (columns, card_w) = grid_columns(ui.available_width());
    let (picture_h, card_h) = card_heights(card_w);
    let dir = app.services.joined_previews_dir();
    let (mut open, mut forget) = (None, None);
    for row in files.chunks(columns.max(1)) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), card_h), Sense::hover());
        for (i, file) in row.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(r.min.x + i as f32 * (card_w + GAP), r.min.y), vec2(card_w, card_h));
            let key = file.preview_key();
            let resp = ui.interact(rect, ui.scope_id().with(("joined", &key)), Sense::click());
            let reach = status(app, file);
            let said = match reach {
                Some(Reach::Online) => tr("Server online"),
                Some(Reach::ServerDown) => tr("Server not reachable"),
                Some(Reach::NoNetwork) => tr("You're offline"),
                None => "",
            };
            let spoken = if said.is_empty() {
                trf("{name}, on {address}", &[("name", &file.name), ("address", &file.address)])
            } else {
                trf("{name}, {status}, on {address}", &[("name", &file.name), ("status", &said), ("address", &file.address)])
            };
            a11y::named_button(&resp, &spoken);
            let resp = resp.on_hover_text(&file.address);
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let state = &mut app.ui.online.joined;
            let pic = state.pictures.entry(key.clone()).or_insert_with(|| {
                let bytes = std::fs::read(preview_file(dir.as_deref()?, &key)).ok()?;
                picture(&ctx, &key, &bytes)
            });
            paint(ui, t, &resp, &Card { file, reach, picture: pic.as_ref(), rect, picture_h, now });
            if resp.clicked() {
                open = Some(file.clone());
            }
            crate::right::context_popup(&resp).show(|ui| {
                if crate::right::menu_item(ui, tr("Forget on this computer"), "") {
                    forget = Some(file.clone());
                    ui.close();
                }
            });
        }
        ui.add_space(GAP);
    }
    if let Some(f) = open
        && let Some(r) = app.run("collab.openJoined", json!({"file": f.file, "address": f.address}))
    {
        if r["alreadyOpen"] == true {
            app.ui.home = false;
        } else {
            app.ui.live.home_join = r["session"].as_u64();
        }
    }
    if let Some(f) = forget {
        app.run("collab.forgetJoined", json!({"file": f.file, "address": f.address}));
        app.services.save_prefs(&app.session.prefs);
        if let Some(dir) = dir {
            let mut forgotten = app.ui.online.joined.forgotten.lock().unwrap_or_else(PoisonError::into_inner);
            forgotten.insert(f.preview_key());
            let _ = std::fs::remove_file(preview_file(&dir, &f.preview_key()));
        }
        app.ui.online.joined.pictures.remove(&f.preview_key());
    }
}
