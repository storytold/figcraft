//! Home ▸ Online: the files you put on a server from this computer (`collab.files`), each with
//! where it is, whether others get in and when it was last opened. A row opens its file as its
//! owner; its menu starts or stops sharing, copies the join code or link, renames it on the
//! server, removes it from the server (confirmed) or forgets it here. Nothing is asked of a
//! server until the section opens or a file is acted on; then each server is asked once, in the
//! background (the answer wakes the window), and one that can't be reached says so while the
//! list shows what this computer remembers.
//!
//! Below them, the team list of the server in Preferences, when its operator keeps one: the files
//! shared from it now by anyone. A row joins its file; its menu copies the file's link (no secret
//! in it) for the app, or for the server's browser build.

use egui::{Align2, CornerRadius, Rect, Sense, Ui, pos2, vec2};
use figcraft_engine::live::utc_date;
use figcraft_engine::online::{FileState, OnlineFile, ServerProblem, server_host};
use figcraft_engine::team::{Team, TeamFile, TeamList, team_link, web_link};
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y::{self, Tip};
use crate::home::{FileRow, file_row};
use crate::i18n::{tr, trf};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, semibold};
use crate::widgets;

const FILES: &str = "collab.files";
const MANAGE: &str = "collab.manageFile";

/// A row's height.
const ROW: f32 = 36.0;

/// UI-only state of the Online section.
#[derive(Default)]
pub struct OnlineUi {
    /// The frame the section last drew in: it asks the servers when it shows again after a gap.
    drawn: Option<u64>,
    /// Remove from server… was chosen for this file (server, id, name): its confirmation shows.
    pub remove: Option<(String, String, String)>,
    /// Rename… was chosen for this file (server, id) and the name being typed.
    pub rename: Option<(String, String, String)>,
    focus_rename: bool,
}

/// What a row says after the file's name.
fn detail(f: &OnlineFile) -> String {
    let host = server_host(&f.server);
    if f.state == FileState::Gone {
        return trf("{server} · No longer on the server", &[("server", &host)]);
    }
    let shared = if f.shared.is_some() { tr("Shared") } else { tr("Not shared") };
    let date = utc_date(f.opened);
    let day = date.get(..10).unwrap_or(&date);
    trf("{server} · {shared} · Opened {date}", &[("server", &host), ("shared", &shared), ("date", &day)])
}

/// The mark of a file others get into (`true`), or one that only its owner does.
pub fn mark(shared: bool) -> Icon {
    if shared { Icon::LiveShared } else { Icon::LivePrivate }
}

/// Ask each server once what became of its files, and the server in Preferences for its team list
/// (in the background).
fn refresh(app: &mut FigApp, ctx: &egui::Context) {
    let ask = !app.session.prefs.online_files.is_empty() || !app.session.prefs.publish_server.trim().is_empty();
    if ask && !app.session.waiting_for(FILES) {
        app.run_background(ctx, FILES, json!({"refresh": true}), Box::new(|_, _, _| {}));
    }
}

/// Run `action` of `collab.manageFile` on recorded file `f` without the window waiting for the
/// server; `done` gets the result.
fn manage(app: &mut FigApp, ctx: &egui::Context, f: (&str, &str), action: &str, extra: Value, done: crate::Done) {
    let mut params = json!({"server": f.0, "file": f.1, "action": action});
    if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    app.run_background(ctx, MANAGE, params, done);
}

/// Open recorded file `f` as its owner; its tab shows once the file arrives.
pub(crate) fn open(app: &mut FigApp, f: &OnlineFile) {
    let Some(r) = app.run("collab.openFile", json!({"server": f.server, "file": f.file})) else { return };
    if r["alreadyOpen"] == true && !r["document"].is_null() {
        app.ui.home = false;
    } else {
        app.ui.live.home_join = r["session"].as_u64();
    }
}

/// The Online section of the home screen, in `main`.
pub fn section(app: &mut FigApp, ui: &mut Ui, main: Rect, t: &Tokens) {
    let ctx = ui.ctx().clone();
    // Shown again after something else was: ask the servers.
    let frame = ctx.cumulative_frame_nr();
    if app.ui.online.drawn.replace(frame).is_none_or(|d| d.saturating_add(1) < frame) {
        refresh(app, &ctx);
    }
    ui.painter().text(main.min, Align2::LEFT_TOP, tr("Online files"), semibold(18.0), t.text);
    let body = Rect::from_min_max(pos2(main.min.x, main.min.y + 40.0), pos2(main.min.x + main.width().min(720.0), main.max.y));
    ui.scope_builder(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.spacing_mut().item_spacing.y = 6.0;
        widgets::label(
            ui,
            tr("The files you shared or published online from this computer. They stay on their server until you remove them."),
            t.text_secondary,
        );
        status(app, ui, t);
        if let Some(session) = app.ui.live.home_join {
            crate::live::join_status(app, ui, t, session);
        }
        let files = app.session.prefs.online_files.clone();
        let team = app.session.team().cloned();
        if files.is_empty() {
            ui.add_space(8.0);
            widgets::label(ui, tr("Nothing online yet. Use Share on a file to put it online and it is listed here."), t.text_secondary);
        }
        ui.add_space(6.0);
        let busy = app.session.waiting_for(MANAGE);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for f in &files {
                row(app, ui, t, f, busy);
            }
            if let Some(team) = &team {
                team_section(app, ui, t, team);
            }
        });
    });
}

/// What the servers are being asked, and which couldn't say.
fn status(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    if app.session.waiting_for(FILES) {
        return widgets::label(ui, tr("Checking the server…"), t.text_secondary);
    }
    if app.session.waiting_for(MANAGE) {
        return widgets::label(ui, tr("Asking the server…"), t.text_secondary);
    }
    let problems: Vec<(String, bool)> =
        app.session.live.servers.iter().map(|(s, p)| (server_host(s).to_string(), matches!(p, ServerProblem::Offline(_)))).collect();
    let mut again = false;
    for (host, offline) in &problems {
        if *offline {
            ui.horizontal(|ui| {
                widgets::label(ui, &trf("Couldn't reach {server}: showing what this computer remembers.", &[("server", host)]), t.danger);
                again |= widgets::button(ui, tr("Try again"), false, true).clicked();
            });
        } else {
            widgets::label(
                ui,
                &trf("{server} can't say what became of your files (it is older than this app).", &[("server", host)]),
                t.text_secondary,
            );
        }
    }
    if again {
        refresh(app, &ui.ctx().clone());
    }
}

/// One file: its mark, name and details; a click opens it, the button at its end has its menu.
fn row(app: &mut FigApp, ui: &mut Ui, t: &Tokens, f: &OnlineFile, busy: bool) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let gone = f.state == FileState::Gone;
    let more = Rect::from_center_size(pos2(r.max.x - 18.0, r.center().y), vec2(24.0, 24.0));
    let details = detail(f);
    a11y::button(&resp, &format!("{}, {details}", f.name));
    let ink = if gone { t.text_tertiary } else { t.text };
    let glyph = if gone { Icon::Close } else { mark(f.shared.is_some()) };
    let tint = if f.shared.is_some() && !gone { t.success } else { t.icon_secondary };
    let shown = FileRow { mark: Some((glyph, tint)), name: &f.name, ink, detail: &details, detail_size: 11.0, detail_ink: t.text_secondary };
    file_row(ui, t, r.with_max_x(more.min.x - 4.0), &resp, &shown);
    // The menu button takes its own clicks.
    let mresp = ui
        .interact(more, ui.scope_id().with(("online_more", &f.server, &f.file)), Sense::click())
        .tip(trf("Actions for {name}", &[("name", &f.name)]));
    a11y::menu_button(&mresp, &trf("Actions for {name}", &[("name", &f.name)]), a11y::menu_open(&mresp));
    if mresp.hovered() {
        ui.painter().rect_filled(more, CornerRadius::same(5), t.pressed);
    }
    icons::paint(ui.painter(), more.shrink(5.0), Icon::Dots, t.icon);
    let ctx = ui.ctx().clone();
    let at = (f.server.as_str(), f.file.as_str());
    crate::right::menu_popup(&mresp).width(240.0).show(|ui| {
        // What asks the server waits its turn: one request about a file at a time.
        let item = |ui: &mut Ui, label: &str, show: bool| show && crate::right::menu_item(ui, label, "");
        if gone {
            if item(ui, tr("Clear from this list"), true) {
                forget(app, f);
                ui.close();
            }
            return;
        }
        if item(ui, tr("Open"), true) {
            open(app, f);
            ui.close();
        }
        let shared = f.shared.as_ref();
        if item(ui, if shared.is_some() { tr("Stop sharing") } else { tr("Start sharing") }, !busy) {
            let said = if shared.is_some() {
                tr("Sharing stopped: the file stays on the server, open only to you")
            } else {
                tr("Shared: copy its code from the file's menu")
            };
            manage(app, &ctx, at, if shared.is_some() { "unshare" } else { "share" }, json!({}), Box::new(move |app, _, _| app.toast(said)));
            ui.close();
        }
        if let Some(code) = shared.and_then(|s| s.edit.code.as_deref())
            && item(ui, tr("Copy code"), true)
        {
            app.clipboard_set_text(&figcraft_engine::figcraft_collab::code::grouped(code));
            app.toast(tr("Join code copied"));
            ui.close();
        }
        if let Some(s) = shared
            && item(ui, tr("Copy link"), true)
        {
            app.clipboard_set_text(&s.edit.link);
            app.toast(tr("Invite link copied"));
            ui.close();
        }
        widgets::separator(ui);
        if item(ui, tr("Rename on the server…"), !busy) {
            app.ui.online.rename = Some((f.server.clone(), f.file.clone(), f.name.clone()));
            app.ui.online.focus_rename = true;
            ui.close();
        }
        if item(ui, tr("Remove from server…"), !busy) {
            app.ui.online.remove = Some((f.server.clone(), f.file.clone(), f.name.clone()));
            ui.close();
        }
        if item(ui, tr("Forget on this computer"), true) {
            forget(app, f);
            ui.close();
        }
    });
    if resp.clicked() && !gone {
        open(app, f);
    }
}

/// The team list of the server in Preferences: its heading, then a row per file shared now (or
/// why there is none). A server without a team list shows nothing.
fn team_section(app: &mut FigApp, ui: &mut Ui, t: &Tokens, team: &Team) {
    let host = server_host(&team.server).to_string();
    let (web, files) = match &team.list {
        TeamList::None => return,
        TeamList::Files { web, files } => (web.as_str(), files.as_slice()),
        TeamList::Offline(_) => ("", &[][..]),
    };
    ui.add_space(14.0);
    ui.label(egui::RichText::new(trf("Shared by your team on {server}", &[("server", &host)])).font(semibold(14.0)).color(t.text));
    match &team.list {
        TeamList::Offline(_) => widgets::label(ui, &trf("Couldn't reach {server} for its team list.", &[("server", &host)]), t.danger),
        _ if files.is_empty() => widgets::label(ui, tr("Nobody shares a file there now."), t.text_secondary),
        _ => {
            widgets::label(ui, tr("Everyone who reaches this server can open these files while they are shared."), t.text_secondary);
            for f in files {
                team_row(app, ui, t, &team.server, web, f);
            }
        }
    }
}

/// What a team row says after the file's name.
fn team_detail(f: &TeamFile) -> String {
    let date = utc_date(f.updated);
    let day = date.get(..10).unwrap_or(&date);
    match f.people {
        0 => trf("Changed {date}", &[("date", &day)]),
        n => trf("{count} in it now · Changed {date}", &[("count", &n.to_string()), ("date", &day)]),
    }
}

/// Join file `f` of the team list on `server`; its tab shows once the file arrives.
fn open_team(app: &mut FigApp, server: &str, f: &TeamFile) {
    if let Some(session) = app.run("collab.join", json!({"link": team_link(server, &f.file)})).and_then(|r| r["session"].as_u64()) {
        app.ui.live.home_join = Some(session);
    }
}

/// One file of the team list: a click joins it; its menu copies its links.
fn team_row(app: &mut FigApp, ui: &mut Ui, t: &Tokens, server: &str, web: &str, f: &TeamFile) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let more = Rect::from_center_size(pos2(r.max.x - 18.0, r.center().y), vec2(24.0, 24.0));
    let details = team_detail(f);
    a11y::button(&resp, &format!("{}, {details}", f.name));
    let shown = FileRow {
        mark: Some((Icon::LiveShared, t.success)),
        name: &f.name,
        ink: t.text,
        detail: &details,
        detail_size: 11.0,
        detail_ink: t.text_secondary,
    };
    file_row(ui, t, r.with_max_x(more.min.x - 4.0), &resp, &shown);
    let mresp =
        ui.interact(more, ui.scope_id().with(("team_more", server, &f.file)), Sense::click()).tip(trf("Actions for {name}", &[("name", &f.name)]));
    a11y::menu_button(&mresp, &trf("Actions for {name}", &[("name", &f.name)]), a11y::menu_open(&mresp));
    if mresp.hovered() {
        ui.painter().rect_filled(more, CornerRadius::same(5), t.pressed);
    }
    icons::paint(ui.painter(), more.shrink(5.0), Icon::Dots, t.icon);
    crate::right::menu_popup(&mresp).width(240.0).show(|ui| {
        if crate::right::menu_item(ui, tr("Open"), "") {
            open_team(app, server, f);
            ui.close();
        }
        if crate::right::menu_item(ui, tr("Copy link"), "") {
            app.clipboard_set_text(&team_link(server, &f.file));
            app.toast(tr("Link copied: it opens the file in FigCraft"));
            ui.close();
        }
        if !web.is_empty() && crate::right::menu_item(ui, tr("Copy browser link"), "") {
            app.clipboard_set_text(&web_link(web, server, &f.file));
            app.toast(tr("Link copied: it opens the file in the browser"));
            ui.close();
        }
    });
    if resp.clicked() {
        open_team(app, server, f);
    }
}

/// Drop `f` from the list on this computer, and say that the file itself stays.
fn forget(app: &mut FigApp, f: &OnlineFile) {
    if app.run("collab.forgetFile", json!({"server": f.server, "file": f.file})).is_none() {
        return;
    }
    if f.state == FileState::Gone {
        app.toast(trf("{name} cleared from the list", &[("name", &f.name)]));
    } else {
        app.toast(trf("{name} is forgotten on this computer; the file stays on {server}", &[("name", &f.name), ("server", &server_host(&f.server))]));
    }
}

/// Escape closes the section's dialogs; true when it did.
pub fn dialog_keys(app: &mut FigApp, ctx: &egui::Context) {
    let o = &mut app.ui.online;
    if (o.remove.is_some() || o.rename.is_some()) && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        o.remove = None;
        o.rename = None;
    }
}

/// The confirmation of Remove from server…, and Rename on the server….
pub fn dialogs(app: &mut FigApp, ctx: &egui::Context) {
    dialog_keys(app, ctx);
    let t = Tokens::get(ctx);
    if let Some((server, file, name)) = app.ui.online.remove.take() {
        let (mut open, mut cancel, mut go) = (true, false, false);
        crate::dialogs::window(tr("Remove from server"), ctx).id(egui::Id::unique("Remove online file")).open(&mut open).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            widgets::label(ui, &trf("Remove {name} from {server}?", &[("name", &name), ("server", &server_host(&server))]), t.text);
            widgets::label(
                ui,
                tr("The file is deleted from the server: nobody can open or join it from there again, and this can't be undone. Copies that are open stay with whoever has them."),
                t.danger,
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                go = widgets::button(ui, tr("Remove"), true, true).clicked();
                cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
            });
        });
        if go {
            let said = trf("{name} was removed from the server", &[("name", &name)]);
            manage(app, ctx, (&server, &file), "remove", json!({}), Box::new(move |app, _, _| app.toast(said)));
        } else if open && !cancel {
            app.ui.online.remove = Some((server, file, name));
        }
    }
    if let Some((server, file, mut name)) = app.ui.online.rename.take() {
        let (mut open, mut cancel, mut go) = (true, false, false);
        let focus = std::mem::take(&mut app.ui.online.focus_rename);
        crate::dialogs::window(tr("Rename on the server"), ctx).id(egui::Id::unique("Rename online file")).open(&mut open).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            widgets::label(ui, tr("The name in your online files and on the server. The document inside keeps its own name."), t.text_secondary);
            let resp = widgets::text_input(ui, egui::Id::unique("online_rename"), &mut name, 380.0, tr("File name"));
            a11y::text(&resp, tr("File name"), &name);
            if focus {
                resp.request_focus();
            }
            go = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                go |= widgets::button(ui, tr("Rename"), true, !name.trim().is_empty()).clicked();
                cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
            });
        });
        if go && !name.trim().is_empty() {
            manage(app, ctx, (&server, &file), "rename", json!({"name": name.trim()}), Box::new(|_, _, _| {}));
        } else if open && !cancel {
            app.ui.online.rename = Some((server, file, name));
        }
    }
}

/// The section's state for `ui.inspect`: no token, no link.
pub fn inspect(app: &FigApp) -> Value {
    let files: Vec<Value> = app
        .session
        .prefs
        .online_files
        .iter()
        .map(|f| json!({"file": f.file, "name": f.name, "detail": detail(f), "shared": f.shared.is_some(), "state": f.state, "hasOwnerToken": true}))
        .collect();
    let team = app.session.team().map(|team| match &team.list {
        TeamList::Files { files, .. } => json!({
            "state": "listed",
            "files": files.iter().map(|f| json!({"file": f.file, "name": f.name, "detail": team_detail(f)})).collect::<Vec<_>>(),
        }),
        TeamList::None => json!({"state": "none", "files": []}),
        TeamList::Offline(_) => json!({"state": "offline", "files": []}),
    });
    json!({
        "open": app.ui.home_view.section == crate::home::Section::Online,
        "files": files,
        "team": team,
        "checking": app.session.waiting_for(FILES),
        "problems": app.session.live.servers.len(),
        "confirmRemove": app.ui.online.remove.as_ref().map(|r| &r.2),
        "rename": app.ui.online.rename.as_ref().map(|r| &r.2),
    })
}
