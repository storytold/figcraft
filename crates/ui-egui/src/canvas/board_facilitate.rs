//! Boards in the UI: the timer and voting pill at the top right of a board, and the vote badges on
//! objects. Everything they change goes through `board.timer` and `board.vote`.

use std::time::Duration;

use egui::{Align2, Color32, CornerRadius, Rect, RichText, Stroke, Ui, pos2, vec2};
use figcraft_doc::{BoardTimer, NodeId};
use figcraft_engine::cmd::facilitate::{self, BADGE_PX};
use serde_json::{Value, json};

use crate::FigApp;
use crate::i18n::{tr, tr_noop, trf, trn};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

const CELL: f32 = 28.0;
/// The presets the timer offers (minutes).
const PRESETS: [u32; 5] = [1, 3, 5, 10, 15];
/// What a vote can be on, as `board.vote {on}` takes it.
const SCOPES: [(&str, &str); 3] = [("all", tr_noop("All objects")), ("stickies", tr_noop("Sticky notes")), ("selection", tr_noop("Selection"))];

/// One object in the results: its rank, id, name, votes and who voted for it (name, votes).
type VoteRow = (usize, NodeId, String, u32, Vec<(String, u32)>);

/// What the pill shows of the vote: none, running (your votes left) or ended (the results).
enum VoteView {
    Off,
    Running { left: u32 },
    Ended { results: Vec<VoteRow> },
}

fn vote_view(app: &FigApp) -> VoteView {
    let Ok(doc) = app.session.doc() else { return VoteView::Off };
    let Some(vote) = &doc.vote else { return VoteView::Off };
    if vote.ended {
        // Once it ends, everyone sees who voted for what.
        let name = |key: &str| facilitate::voter_name(&app.session, key).unwrap_or_else(|| tr("Someone who left").to_string());
        let voters = |id| facilitate::voters_of(doc, vote, id).into_iter().map(|(key, n)| (name(&key), n)).collect();
        let results =
            facilitate::ranked(doc, vote).into_iter().map(|(rank, id, n)| (rank, id, facilitate::object_name(doc, id), n, voters(id))).collect();
        return VoteView::Ended { results };
    }
    let used: u32 = facilitate::mine(doc, &facilitate::voter(&app.session, &Value::Null)).iter().map(|m| m.1).sum();
    VoteView::Running { left: vote.per_person.saturating_sub(used) }
}

/// The pill at the top right of a board's canvas: the timer, then the vote.
pub fn show(app: &mut FigApp, ctx: &egui::Context, canvas: Rect) {
    let Ok(doc) = app.session.doc() else { return };
    if !figcraft_engine::board::is_board(doc) {
        return;
    }
    let timer = doc.timer;
    let now = facilitate::now_ms();
    if let Some(left) = timer.filter(|t| matches!(t, BoardTimer::Running { .. })).map(|t| t.left(now)).filter(|l| *l > 0) {
        // Wake when the shown second changes; nothing else ticks.
        ctx.request_repaint_after(Duration::from_millis(left % 1000 + 5));
    }
    let vote = vote_view(app);
    let editable = app.session.active_read_only().is_none();
    let t = Tokens::get(ctx);
    egui::Area::new(egui::Id::unique("board_facilitate"))
        .order(egui::Order::Middle)
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(pos2(canvas.max.x - 12.0, canvas.min.y + 12.0))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.panel)
                .stroke(Stroke::new(1.0, t.border))
                .corner_radius(CornerRadius::same(10))
                .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 12, spread: 0, color: t.shadow })
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        timer_part(app, ui, timer, now, editable, &t);
                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(4.0);
                        vote_part(app, ui, vote, editable, &t);
                    });
                });
        });
}

fn timer_part(app: &mut FigApp, ui: &mut Ui, timer: Option<BoardTimer>, now: u64, editable: bool, t: &Tokens) {
    let Some(timer) = timer else {
        let resp = icons::button(ui, Icon::Timer, CELL, false, tr("Timer"));
        if editable {
            timer_popup(app, &resp);
        }
        return;
    };
    let state = facilitate::timer_state(Some(timer), now);
    let color = match state {
        "done" => t.danger,
        "paused" => t.text_secondary,
        _ => t.text,
    };
    let (r, _) = ui.allocate_exact_size(vec2(20.0, CELL), egui::Sense::hover());
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), Icon::Timer, color);
    // As wide as the same clock in zeros: the pill doesn't jitter as the seconds change.
    let text = facilitate::clock(timer.left(now));
    let zeros: String = text.chars().map(|c| if c.is_ascii_digit() { '0' } else { c }).collect();
    let w = ui.painter().layout_no_wrap(zeros, semibold(13.0), color).size().x;
    let (r, _) = ui.allocate_exact_size(vec2(w + 2.0, CELL), egui::Sense::hover());
    ui.painter().text(r.left_center(), Align2::LEFT_CENTER, text, semibold(13.0), color);
    ui.add_space(2.0);
    if !editable {
        return;
    }
    let (icon, tip, action) = match timer {
        BoardTimer::Paused { .. } => (Icon::Play, tr("Resume"), "resume"),
        BoardTimer::Running { .. } => (Icon::Pause, tr("Pause"), "pause"),
    };
    if state != "done" && icons::button(ui, icon, CELL, false, tip).clicked() {
        app.run("board.timer", json!({"action": action}));
    }
    if icons::button(ui, Icon::Plus, CELL, false, tr("Add a minute")).clicked() {
        app.run("board.timer", json!({"action": "add", "minutes": 1}));
    }
    if icons::button(ui, Icon::Close, CELL, false, tr("Stop timer")).clicked() {
        app.run("board.timer", json!({"action": "stop"}));
    }
}

/// The timer button's popover: presets and a custom length.
fn timer_popup(app: &mut FigApp, anchor: &egui::Response) {
    crate::right::menu_popup(anchor).width(300.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 8.0);
        ui.add_space(2.0);
        ui.label(RichText::new(tr("Timer")).font(semibold(12.0)));
        let mut start = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for m in PRESETS {
                if widgets::button(ui, &trf("{n} min", &[("n", &m)]), false, true).clicked() {
                    start = Some(m);
                }
            }
        });
        ui.horizontal(|ui| {
            let id = egui::Id::unique("board_timer_minutes");
            let mut minutes: u32 = ui.data(|d| d.get_temp(id)).unwrap_or(5);
            widgets::label(ui, tr("Minutes"), Tokens::get(ui.ctx()).text_secondary);
            ui.add(egui::DragValue::new(&mut minutes).range(1..=1440).speed(0.25));
            ui.data_mut(|d| d.insert_temp(id, minutes));
            if widgets::button(ui, tr("Start"), true, true).clicked() {
                start = Some(minutes);
            }
        });
        if let Some(minutes) = start {
            app.run("board.timer", json!({"action": "start", "minutes": minutes}));
            ui.close();
        }
    });
}

fn vote_part(app: &mut FigApp, ui: &mut Ui, vote: VoteView, editable: bool, t: &Tokens) {
    match vote {
        VoteView::Off => {
            let resp = icons::button(ui, Icon::Vote, CELL, false, tr("Voting session"));
            if editable {
                vote_popup(app, &resp);
            }
        }
        VoteView::Running { left } => {
            let (r, resp) = ui.allocate_exact_size(vec2(20.0, CELL), egui::Sense::hover());
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), Icon::Vote, t.accent_text);
            let hint = tr("Click objects to vote; click your badge to take a vote back");
            resp.on_hover_text(hint);
            ui.label(RichText::new(trn("{n} votes left", left as usize, &[])).font(regular(12.0)).color(t.text)).on_hover_text(hint);
            ui.add_space(4.0);
            if editable && widgets::button(ui, tr("End vote"), true, true).clicked() {
                app.run("board.vote", json!({"action": "end"}));
            }
        }
        VoteView::Ended { results } => {
            let (r, _) = ui.allocate_exact_size(vec2(20.0, CELL), egui::Sense::hover());
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), Icon::Vote, t.accent_text);
            let resp = widgets::button(ui, tr("Results"), false, true);
            results_popup(app, &resp, &results, editable);
        }
    }
}

/// The vote button's popover: votes per person, what to vote on, Start.
fn vote_popup(app: &mut FigApp, anchor: &egui::Response) {
    let selected = !app.session.selection().is_empty();
    crate::right::menu_popup(anchor).width(236.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.add_space(2.0);
        ui.label(RichText::new(tr("Voting session")).font(semibold(12.0)));
        let (votes_id, on_id) = (egui::Id::unique("board_vote_votes"), egui::Id::unique("board_vote_on"));
        let mut votes: u32 = ui.data(|d| d.get_temp(votes_id)).unwrap_or(3);
        let mut on: usize = ui.data(|d| d.get_temp(on_id)).unwrap_or(0);
        if on == 2 && !selected {
            on = 0;
        }
        ui.horizontal(|ui| {
            widgets::label(ui, tr("Votes per person"), Tokens::get(ui.ctx()).text_secondary);
            ui.add(egui::DragValue::new(&mut votes).range(1..=99));
        });
        for (i, (_, label)) in SCOPES.iter().enumerate() {
            // The selection is offered while there is one.
            if (i != 2 || selected) && widgets::radio(ui, on == i, tr(label)) {
                on = i;
            }
        }
        ui.data_mut(|d| {
            d.insert_temp(votes_id, votes);
            d.insert_temp(on_id, on);
        });
        let scope = SCOPES.get(on).map_or("all", |s| s.0);
        if widgets::button(ui, tr("Start vote"), true, true).clicked() {
            app.run("board.vote", json!({"action": "start", "votes": votes, "on": scope}));
            ui.close();
        }
    });
}

/// The ended vote's results, most votes first, each with who voted for it: a click on a row
/// selects that object.
fn results_popup(app: &mut FigApp, anchor: &egui::Response, results: &[VoteRow], editable: bool) {
    crate::right::menu_popup(anchor).width(260.0).show(|ui| {
        ui.add_space(2.0);
        ui.label(RichText::new(tr("Results")).font(semibold(12.0)));
        let secondary = Tokens::get(ui.ctx()).text_secondary;
        if results.is_empty() {
            widgets::label(ui, tr("No votes yet"), secondary);
        }
        for (rank, id, name, n, voters) in results {
            let label = format!("{rank}. {}", widgets::elide(ui, name, 160.0));
            if crate::right::menu_item(ui, &label, &trn("{n} votes", *n as usize, &[])) {
                app.run("select.set", json!({"ids": [id]}));
            }
            // "Ada, Grace ×2": a person with several votes on it says how many.
            let who: Vec<String> = voters
                .iter()
                .map(|(name, n)| if *n > 1 { trf("{name} ×{n}", &[("name", name), ("n", &n.to_string())]) } else { name.clone() })
                .collect();
            let full = who.join(", ");
            let shown = widgets::elide(ui, &full, 236.0);
            // Close under its row, apart from the next one.
            ui.add_space(-8.0);
            let resp = ui.horizontal(|ui| {
                ui.add_space(8.0);
                widgets::label(ui, &shown, secondary);
            });
            ui.add_space(6.0);
            if shown != full {
                resp.response.on_hover_text(full);
            }
        }
        if editable {
            ui.add_space(4.0);
            if widgets::button(ui, tr("Clear results"), false, true).clicked() {
                app.run("board.vote", json!({"action": "clear"}));
                ui.close();
            }
        }
    });
}

/// The vote badges on the page: your votes while the vote runs, everyone's once it ends.
pub fn badges(app: &FigApp, painter: &egui::Painter) {
    let Ok(ds) = app.session.ds() else { return };
    let doc = &ds.doc;
    let Some(vote) = &doc.vote else { return };
    let counts = if vote.ended { vote.tally(doc, &doc.votes) } else { facilitate::mine(doc, &facilitate::voter(&app.session, &Value::Null)) };
    let t = Tokens::get(painter.ctx());
    let r = BADGE_PX as f32;
    for (id, n) in counts {
        let Some(b) = doc.world_bounds(id).filter(|_| doc.page_of(id) == Some(ds.page)) else { continue };
        let (c, _) = facilitate::badge(b, ds.view.zoom);
        let c = app.to_screen(c.x, c.y);
        painter.circle_filled(c + vec2(0.0, 1.0), r + 1.0, Color32::from_black_alpha(40));
        painter.circle_filled(c, r, t.accent);
        painter.circle_stroke(c, r, Stroke::new(1.5, Color32::WHITE));
        painter.text(c, Align2::CENTER_CENTER, n.to_string(), semibold(11.0), t.accent_on);
    }
}
