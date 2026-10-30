use super::{
    Align2, Color32, CornerRadius, FigApp, Id, MENU, Rect, SEP, Sense, Stroke, StrokeKind, Tokens, Ui, available, i18n, invoke, json, pos2,
    pretty_shortcut, record_recent, regular, tr, vec2,
};

/// The Actions palette's tab.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum PaletteTab {
    #[default]
    All,
    Assets,
    Plugins,
}

/// The Actions palette's content height (the frame adds its own 8 px margins).
const PALETTE_H: f32 = 450.0;

/// Open the Actions palette afresh (empty search, All tab, first row highlighted), or close it.
pub fn set_palette(ui: &mut crate::UiState, open: bool) {
    ui.palette = open.then(String::new);
    ui.palette_tab = PaletteTab::All;
    ui.palette_hi = 0;
    ui.palette_armed = open;
}

/// Open the Actions palette afresh, or close it when it's open (⌘K, the toolbar's Actions button).
pub fn toggle_palette(ui: &mut crate::UiState) {
    let open = ui.palette.is_none();
    set_palette(ui, open);
}

/// The Actions palette: search over commands, assets and plugins, with tabs, recents and grouped
/// sections, floating above the toolbar. Rows run on click; Up and Down move the highlight and
/// Enter runs the highlighted row.
pub(super) fn palette(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut q) = app.ui.palette.clone() else { return };
    let t = Tokens::get(ctx);
    // Same centre and width as the toolbar pill (it centres on the canvas, not the window),
    // floating 8 px above it. Fixed size: switching tabs never resizes the panel.
    let bar_w = crate::toolbar::bar_width(app);
    let bar = crate::toolbar::pill_rect(app.canvas.rect, bar_w);
    let area = Rect::from_min_max(pos2(bar.min.x, bar.min.y - 8.0 - (PALETTE_H + 16.0)), pos2(bar.max.x, bar.min.y - 8.0));
    let mut run: Option<String> = None;
    let mut close = false;
    // A press anywhere outside closes the panel. The opening press is spared once, and the
    // spare expires the first frame without a press (so the next outside press always closes).
    let pressed = ctx.input(|i| i.pointer.any_pressed());
    if pressed && !std::mem::replace(&mut app.ui.palette_armed, false) {
        let on_toggle = app.ui.actions_rect.is_some_and(|b| ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| b.contains(p))));
        let outside = !on_toggle && ctx.input(|i| i.pointer.hover_pos().map(|p| !area.contains(p)).unwrap_or(false));
        if outside {
            close = true;
        }
    } else if !pressed {
        app.ui.palette_armed = false;
    }
    // Taken before the search field sees them (it would move its cursor).
    let step = ctx.input_mut(|i| {
        i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) as i64
            - i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) as i64
    });
    egui::Area::new(Id::unique("palette")).fixed_pos(area.min).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .corner_radius(CornerRadius::same(13))
            .stroke(Stroke::new(1.0, t.border))
            .shadow(egui::Shadow { offset: [0, 8], blur: 32, spread: 0, color: t.shadow })
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                // Same outer width as the toolbar pill, and a fixed height: switching tabs
                // never resizes the panel.
                let inner = bar_w - 16.0;
                ui.set_width(inner);
                ui.set_height(PALETTE_H);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut q)
                        .hint_text(tr("Search actions, layers and tools"))
                        .desired_width(inner - 16.0)
                        .font(regular(14.0))
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(8))),
                );
                r.request_focus();
                crate::widgets::select_all_on_focus(ui, &r, &q);
                // A new search starts again at the first row.
                if r.changed() {
                    app.ui.palette_hi = 0;
                }
                ui.add_space(4.0);
                // Tabs: All, Assets, Plugins & widgets.
                let tabs = [(PaletteTab::All, tr("All")), (PaletteTab::Assets, tr("Assets")), (PaletteTab::Plugins, tr("Plugins & widgets"))];
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
                    for (tab, label) in tabs {
                        let active = app.ui.palette_tab == tab;
                        // The frame stays on for every tab (transparent when idle) so the
                        // active pill never moves its neighbours.
                        let resp = ui.add(
                            egui::Button::new(egui::RichText::new(label).size(12.0).color(if active { t.text } else { t.text_secondary }))
                                .frame(true)
                                .fill(if active { t.hover } else { Color32::TRANSPARENT })
                                .corner_radius(CornerRadius::same(6)),
                        );
                        if resp.clicked() {
                            app.ui.palette_tab = tab;
                            app.ui.palette_hi = 0;
                        }
                    }
                });
                ui.add_space(4.0);
                let sections = palette_sections(app, &q, app.ui.palette_tab);
                let count = sections.iter().map(|(_, rows)| rows.len()).sum::<usize>();
                // At either end it stays where it was.
                let hi = (app.ui.palette_hi as i64 + step).clamp(0, count.saturating_sub(1) as i64) as usize;
                app.ui.palette_hi = hi;
                // The pointer moving over a row moves the highlight there.
                let pointer_moved = ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    let mut i = 0;
                    for (name, rows) in &sections {
                        section(ui, &t, name, |ui| {
                            for row in rows {
                                let resp = draw_row(ui, &t, row, i == hi);
                                if resp.clicked() {
                                    run = Some(row.key.clone());
                                }
                                if pointer_moved && resp.hovered() && i != hi {
                                    app.ui.palette_hi = i;
                                    // Drawn there on the next frame, even if the pointer stops now.
                                    ui.ctx().request_repaint();
                                }
                                if step != 0 && i == hi {
                                    resp.scroll_to_me(None);
                                }
                                i += 1;
                            }
                        });
                    }
                });
                if run.is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    run = sections.iter().flat_map(|(_, rows)| rows).nth(hi).map(|row| row.key.clone());
                }
            });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if let Some(key) = run {
        app.ui.palette = None;
        run_action(app, &key);
        return;
    }
    app.ui.palette = if close { None } else { Some(q) };
    let _ = StrokeKind::Inside;
}

/// The rows a palette tab shows for the query, by section: All has Recents, the commands by menu,
/// then plugins and widgets; Assets this file's components and styles; Plugins & widgets those.
pub(crate) fn palette_sections(app: &FigApp, q: &str, tab: PaletteTab) -> Vec<(String, Vec<ActionRow>)> {
    let mut out = match tab {
        PaletteTab::All => {
            let recents = app.session.prefs.recent_actions.iter().filter_map(|key| recent_row(app, key, q)).collect();
            let mut out = vec![(tr("Recents").to_string(), recents)];
            out.extend(command_sections(q));
            out
        }
        PaletteTab::Assets => asset_sections(app, q),
        PaletteTab::Plugins => Vec::new(),
    };
    if tab != PaletteTab::Assets {
        out.push((tr("Plugins").to_string(), plugin_rows(app, q)));
        out.push((tr("Widgets").to_string(), widget_rows(app, q)));
    }
    out.retain(|(_, rows)| !rows.is_empty());
    out
}

/// Run a palette row: `command:<id>`, `plugin:<plugin>/<command>`, `widget:<plugin>`,
/// `component:<node>` or `style:<id>`.
pub(crate) fn run_action(app: &mut FigApp, key: &str) {
    if let Some(id) = key.strip_prefix("command:") {
        record_recent(app, key);
        invoke(app, id);
    } else if let Some(rest) = key.strip_prefix("plugin:") {
        if let Some((plugin, command)) = rest.split_once('/') {
            record_recent(app, key);
            crate::plugins_ui::start(app, plugin, command);
        }
    } else if let Some(plugin) = key.strip_prefix("widget:") {
        record_recent(app, key);
        app.run("widget.insert", json!({ "plugin": plugin }));
    } else if let Some(node) = key.strip_prefix("component:") {
        if let Ok(n) = node.parse::<u64>() {
            let (cx, cy) = app.session.ds().map(|d| (d.view.cx, d.view.cy)).unwrap_or((0.0, 0.0));
            app.run("instance.place", json!({ "component": n, "x": cx, "y": cy }));
        }
    } else if let Some(style) = key.strip_prefix("style:") {
        if app.session.selection().is_empty() {
            app.toast(tr("Select layers to apply the style to"));
        } else {
            app.run("style.apply", json!({ "style": style }));
        }
    }
}

/// One pickable palette row.
pub(crate) struct ActionRow {
    pub(crate) key: String,
    pub(crate) label: String,
    hint: String,
    /// A letter tile instead of an icon (plugins carry no artwork).
    tile: Option<char>,
}

/// A section header with its rows.
fn section(ui: &mut Ui, t: &Tokens, name: &str, rows: impl FnOnce(&mut Ui)) {
    ui.add_space(10.0);
    ui.painter().text(pos2(ui.min_rect().min.x + 10.0, ui.cursor().min.y + 8.0), Align2::LEFT_TOP, name, regular(12.5), t.text_secondary);
    ui.add_space(22.0);
    rows(ui);
}

/// Draw one row (a letter tile for plugins, else just the label) with its right-side hint,
/// `highlighted` when Enter runs it.
fn draw_row(ui: &mut Ui, t: &Tokens, row: &ActionRow, highlighted: bool) -> egui::Response {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
    // The highlight pill stays inside the row: it never touches the panel's edges.
    let pill = r.shrink2(vec2(4.0, 1.0));
    if highlighted {
        ui.painter().rect_filled(pill, CornerRadius::same(6), t.hover);
    }
    let mut label_x = r.min.x + 10.0;
    if let Some(letter) = row.tile {
        let tile = Rect::from_center_size(pos2(r.min.x + 20.0, r.center().y), vec2(18.0, 18.0));
        ui.painter().rect_filled(tile, CornerRadius::same(5), t.hover);
        ui.painter().text(tile.center(), Align2::CENTER_CENTER, letter.to_string(), regular(11.0), t.text);
        label_x = r.min.x + 38.0;
    }
    ui.painter().text(pos2(label_x, r.center().y), Align2::LEFT_CENTER, &row.label, regular(13.0), t.text);
    ui.painter().text(pos2(r.max.x - 10.0, r.center().y), Align2::RIGHT_CENTER, &row.hint, regular(11.0), t.text_secondary);
    resp
}

/// Whether `label` (or `extra`) matches the query, in any case.
fn matches(q: &str, label: &str, extra: &str) -> bool {
    q.is_empty() || label.to_lowercase().contains(q) || extra.to_lowercase().contains(q)
}

/// A command's row: its label, and its shortcut as the hint.
fn command_row(id: &str, label: String) -> ActionRow {
    ActionRow { key: format!("command:{id}"), label, hint: crate::shortcuts::shortcut(id).map(pretty_shortcut).unwrap_or_default(), tile: None }
}

/// Most rows a search shows (the best matches).
const MAX_HITS: usize = 60;

/// Commands grouped by top-level menu, filtered by the query. Without one, the menus' items in
/// catalog order; a search covers every command (the engine's and the UI's own, "Other" for those
/// no menu lists, except those that need parameters or read like a menu item), best matches first
/// (labels starting with the query, then the shortest), each group placed by its best row. Labels
/// match in the UI language and in English. The Plugins menu's items come with the plugins
/// ([`plugin_rows`]).
pub(crate) fn command_sections(q: &str) -> Vec<(String, Vec<ActionRow>)> {
    let ql = q.trim().to_lowercase();
    let mut menu_of = std::collections::HashMap::new();
    for m in MENU.iter().filter(|m| m.label != SEP) {
        menu_of.entry(m.id).or_insert(m);
    }
    let menu_items = MENU.iter().filter(|m| m.label != SEP).map(|m| (m.id, m.label));
    // A row runs its command without parameters (as its menu item does, dialogs included): one
    // that needs them is offered only through a menu item.
    let engine = figcraft_engine::all_commands().iter().filter(|c| !figcraft_engine::cmd::needs_params(c.id)).map(|c| (c.id, c.label));
    let candidates: Box<dyn Iterator<Item = (&str, &str)>> = if ql.is_empty() { Box::new(menu_items) } else { Box::new(menu_items.chain(engine)) };
    let mut seen = std::collections::HashSet::new();
    // The labels menu items show: a command no menu lists that reads the same is left out.
    let mut menu_labels = std::collections::HashSet::new();
    // By top-level menu; `None`: no menu lists it.
    let mut hits: Vec<(Option<&str>, ActionRow)> = Vec::new();
    for (id, english) in candidates {
        if !available(id) || !seen.insert(id) {
            continue;
        }
        let menu = menu_of.get(id);
        let group = menu.map(|m| m.menu.split('/').next().unwrap_or(m.menu));
        if group == Some("Plugins") {
            continue;
        }
        let label = match menu {
            Some(m) => i18n::menu_item(id, m.label),
            None => i18n::command(id, english),
        };
        let same = same_label(&label);
        if menu.is_some() {
            menu_labels.insert(same);
        } else if menu_labels.contains(&same) {
            continue;
        }
        if matches(&ql, &label, &format!("{english} {id}")) {
            hits.push((group, command_row(id, label.into_owned())));
        }
    }
    if !ql.is_empty() {
        hits.sort_by_cached_key(|(_, row)| (!row.label.to_lowercase().starts_with(&ql), row.label.chars().count()));
        hits.truncate(MAX_HITS);
    }
    let mut groups: Vec<(String, Vec<ActionRow>)> = Vec::new();
    for (group, row) in hits {
        let group = group.map_or_else(|| tr("Other"), tr).to_string();
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, rows)) => rows.push(row),
            None => groups.push((group, vec![row])),
        }
    }
    groups
}

/// `label` as two rows that read the same compare: any case, without a trailing ellipsis.
fn same_label(label: &str) -> String {
    label.trim().trim_end_matches(['…', '.']).trim_end().to_lowercase()
}

/// Installed plugins' commands, filtered by the query.
fn plugin_rows(app: &FigApp, q: &str) -> Vec<ActionRow> {
    let ql = q.to_lowercase();
    let mut rows = Vec::new();
    for p in &app.session.prefs.plugins {
        let m = &p.manifest;
        for c in &m.commands {
            let label = if m.commands.len() == 1 { m.name.clone() } else { format!("{} — {}", m.name, c.label) };
            if !matches(&ql, &label, &m.id) {
                continue;
            }
            rows.push(ActionRow {
                key: format!("plugin:{}/{}", m.id, c.id),
                label,
                hint: if c.params.is_empty() { tr("Plugin").to_string() } else { c.label.clone() },
                tile: m.name.chars().next(),
            });
        }
    }
    // The Plugins menu's own items (run last, manage, import).
    for m in MENU.iter().filter(|m| m.label != SEP && m.menu.split('/').next() == Some("Plugins") && available(m.id)) {
        let label = i18n::menu_item(m.id, m.label);
        if matches(&ql, &label, &format!("{} {}", m.label, m.id)) {
            rows.push(command_row(m.id, label.into_owned()));
        }
    }
    rows
}

/// Installed widgets, filtered by the query.
fn widget_rows(app: &FigApp, q: &str) -> Vec<ActionRow> {
    let ql = q.to_lowercase();
    app.session
        .prefs
        .plugins
        .iter()
        .filter(|p| p.manifest.widget)
        .filter(|p| matches(&ql, &p.manifest.name, &p.manifest.id))
        .map(|p| ActionRow {
            key: format!("widget:{}", p.manifest.id),
            label: p.manifest.name.clone(),
            hint: tr("Widget").to_string(),
            tile: p.manifest.name.chars().next(),
        })
        .collect()
}

/// This file's components and styles, filtered by the query.
fn asset_sections(app: &FigApp, q: &str) -> Vec<(String, Vec<ActionRow>)> {
    let ql = q.to_lowercase();
    let Ok(doc) = app.session.doc() else { return Vec::new() };
    let components: Vec<ActionRow> = figcraft_engine::library::local_components(doc)
        .iter()
        .filter_map(|id| doc.node(*id))
        .filter(|n| matches(&ql, &n.name, ""))
        .map(|n| ActionRow { key: format!("component:{}", n.id.0), label: n.name.clone(), hint: tr("Component").to_string(), tile: None })
        .collect();
    let styles: Vec<ActionRow> = figcraft_engine::library::local_styles(doc)
        .filter(|s| matches(&ql, &s.name, ""))
        .map(|s| ActionRow { key: format!("style:{}", s.id), label: s.name.clone(), hint: tr("Style").to_string(), tile: None })
        .collect();
    let mut out = Vec::new();
    if !components.is_empty() {
        out.push((tr("Components").to_string(), components));
    }
    if !styles.is_empty() {
        out.push((tr("Styles").to_string(), styles));
    }
    out
}

/// A recent action back as a row (gone commands, plugins or assets drop out).
pub(crate) fn recent_row(app: &FigApp, key: &str, q: &str) -> Option<ActionRow> {
    let ql = q.to_lowercase();
    if let Some(id) = key.strip_prefix("command:") {
        let spec = figcraft_engine::find_command(id);
        let menu_label = figcraft_engine::catalog::MENU.iter().find(|m| m.id == id);
        let english = menu_label.map(|m| m.label).or_else(|| spec.map(|s| s.label)).unwrap_or(id);
        let label = i18n::command(id, english);
        if !available(id) || !matches(&ql, &label, &format!("{english} {id}")) {
            return None;
        }
        return Some(command_row(id, label.into_owned()));
    }
    if let Some(rest) = key.strip_prefix("plugin:") {
        let (pid, cid) = rest.split_once('/')?;
        let p = app.session.prefs.plugins.iter().find(|p| p.manifest.id == pid)?;
        let c = p.manifest.commands.iter().find(|c| c.id == cid)?;
        let label = if p.manifest.commands.len() == 1 { p.manifest.name.clone() } else { format!("{} — {}", p.manifest.name, c.label) };
        if !matches(&ql, &label, pid) {
            return None;
        }
        return Some(ActionRow { key: key.to_string(), label, hint: tr("Plugin").to_string(), tile: p.manifest.name.chars().next() });
    }
    if let Some(pid) = key.strip_prefix("widget:") {
        let p = app.session.prefs.plugins.iter().find(|p| p.manifest.id == pid && p.manifest.widget)?;
        if !matches(&ql, &p.manifest.name, pid) {
            return None;
        }
        return Some(ActionRow {
            key: key.to_string(),
            label: p.manifest.name.clone(),
            hint: tr("Widget").to_string(),
            tile: p.manifest.name.chars().next(),
        });
    }
    None
}
