//! The About window's credits. GitHub supplies current contributor usernames, commit counts and
//! merged PR authors on demand; the built-in snapshot remains available without a connection.
//! Display and real names in the snapshot are included only with recorded consent.

use std::cmp::Ordering;

use egui::RichText;

use crate::i18n::{tr, tr_noop, trf};
use crate::theme::{Tokens, semibold};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contributor {
    pub login: &'static str,
    pub display_name: Option<&'static str>,
    pub real_name: Option<&'static str>,
    pub prs: u64,
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
    pub binary_added: u64,
    pub binary_deleted: u64,
    /// ISO 8601 UTC; empty for someone credited only through a merged PR.
    pub first_commit: &'static str,
    pub last_commit: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub company: &'static str,
    pub model: &'static str,
    pub version: &'static str,
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
}

include!(concat!(env!("OUT_DIR"), "/credits.rs"));

/// Which name to show. Display and real names fall back to the `@username` when not given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NameMode {
    #[default]
    Username,
    DisplayName,
    RealName,
}

impl NameMode {
    pub const ALL: [NameMode; 3] = [NameMode::Username, NameMode::DisplayName, NameMode::RealName];

    pub fn label(self) -> &'static str {
        match self {
            NameMode::Username => tr("Username"),
            NameMode::DisplayName => tr("Display name"),
            NameMode::RealName => tr("Real name"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Prs,
    Commits,
    LinesAdded,
    LinesDeleted,
    LinesDelta,
    BinaryAdded,
    BinaryDeleted,
    #[default]
    FirstCommit,
    LastCommit,
}

impl SortKey {
    pub const ALL: [SortKey; 10] = [
        SortKey::Name,
        SortKey::Prs,
        SortKey::Commits,
        SortKey::LinesAdded,
        SortKey::LinesDeleted,
        SortKey::LinesDelta,
        SortKey::BinaryAdded,
        SortKey::BinaryDeleted,
        SortKey::FirstCommit,
        SortKey::LastCommit,
    ];

    /// Menu label and table header.
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            SortKey::Name => (tr("Name (A–Z)"), tr("Name")),
            SortKey::Prs => (tr("Merged PRs"), "PRs"),
            SortKey::Commits => (tr("Commits"), tr("Commits")),
            SortKey::LinesAdded => (tr("Lines added"), "+LOC"),
            SortKey::LinesDeleted => (tr("Lines deleted"), "−LOC"),
            SortKey::LinesDelta => (tr("Line delta"), "ΔLOC"),
            SortKey::BinaryAdded => (tr("Binary assets added"), "+Bin"),
            SortKey::BinaryDeleted => (tr("Binary assets removed"), "−Bin"),
            SortKey::FirstCommit => (tr("First commit"), tr("First")),
            SortKey::LastCommit => (tr("Last commit"), tr("Last")),
        }
    }

    /// Names and dates read naturally oldest/A first; counts read biggest first.
    pub fn default_ascending(self) -> bool {
        matches!(self, SortKey::Name | SortKey::FirstCommit | SortKey::LastCommit)
    }

    /// Counts line up on the right of their column; names and dates read from the left.
    fn numeric(self) -> bool {
        !self.default_ascending()
    }

    /// The table cell for `c` (the name column shows `c.name(..)` as a link instead).
    fn cell(self, c: &Contributor) -> String {
        match self {
            SortKey::Name => c.login.to_string(),
            SortKey::Prs => group(c.prs),
            SortKey::Commits => group(c.commits),
            SortKey::LinesAdded => group(c.lines_added),
            SortKey::LinesDeleted => group(c.lines_deleted),
            SortKey::LinesDelta => signed(c.lines_delta()),
            SortKey::BinaryAdded => group(c.binary_added),
            SortKey::BinaryDeleted => group(c.binary_deleted),
            SortKey::FirstCommit => day(c.first_commit).to_string(),
            SortKey::LastCommit => day(c.last_commit).to_string(),
        }
    }
}

impl Contributor {
    pub fn name(&self, mode: NameMode) -> String {
        let chosen = match mode {
            NameMode::Username => None,
            NameMode::DisplayName => self.display_name,
            NameMode::RealName => self.real_name,
        };
        chosen.map_or_else(|| format!("@{}", self.login), str::to_string)
    }

    pub fn lines_delta(&self) -> i128 {
        i128::from(self.lines_added) - i128::from(self.lines_deleted)
    }

    /// One line with everything we know, for tooltips.
    pub fn summary(&self) -> String {
        trf(
            "@{login}: {prs} PRs, {commits} commits, +{added} / −{deleted} lines (Δ {delta}), +{bin_added} / −{bin_deleted} binary assets, {first} – {last}",
            &[
                ("login", &self.login),
                ("prs", &group(self.prs)),
                ("commits", &group(self.commits)),
                ("added", &group(self.lines_added)),
                ("deleted", &group(self.lines_deleted)),
                ("delta", &signed(self.lines_delta())),
                ("bin_added", &group(self.binary_added)),
                ("bin_deleted", &group(self.binary_deleted)),
                ("first", &day(self.first_commit)),
                ("last", &day(self.last_commit)),
            ],
        )
    }
}

/// Alphabetical key: case-insensitive, ignoring the `@` of usernames.
fn name_key(name: &str) -> String {
    name.trim_start_matches('@').to_lowercase()
}

fn compare(a: &Contributor, b: &Contributor, mode: NameMode, key: SortKey) -> Ordering {
    match key {
        SortKey::Name => name_key(&a.name(mode)).cmp(&name_key(&b.name(mode))),
        SortKey::Prs => a.prs.cmp(&b.prs),
        SortKey::Commits => a.commits.cmp(&b.commits),
        SortKey::LinesAdded => a.lines_added.cmp(&b.lines_added),
        SortKey::LinesDeleted => a.lines_deleted.cmp(&b.lines_deleted),
        SortKey::LinesDelta => a.lines_delta().cmp(&b.lines_delta()),
        SortKey::BinaryAdded => a.binary_added.cmp(&b.binary_added),
        SortKey::BinaryDeleted => a.binary_deleted.cmp(&b.binary_deleted),
        // A PR-only contributor (no commit date) sorts after everyone with a date.
        SortKey::FirstCommit => dated(a.first_commit).cmp(&dated(b.first_commit)),
        SortKey::LastCommit => dated(a.last_commit).cmp(&dated(b.last_commit)),
    }
}

fn dated(d: &str) -> (bool, &str) {
    (d.is_empty(), d)
}

/// Contributors in display order. Ties fall back to the name (A–Z) so the order is stable.
pub fn sorted(list: &[Contributor], mode: NameMode, key: SortKey, ascending: bool) -> Vec<&Contributor> {
    let mut v: Vec<&Contributor> = list.iter().collect();
    v.sort_by(|a, b| {
        let o = compare(a, b, mode, key);
        let o = if ascending { o } else { o.reverse() };
        o.then_with(|| compare(a, b, mode, SortKey::Name)).then_with(|| a.login.cmp(b.login))
    });
    v
}

fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn signed(n: i128) -> String {
    let g = group(u64::try_from(n.unsigned_abs()).unwrap_or(u64::MAX));
    if n < 0 { format!("−{g}") } else { format!("+{g}") }
}

fn day(iso: &str) -> &str {
    if iso.is_empty() { "—" } else { iso.get(..10).unwrap_or(iso) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    names: NameMode,
    key: SortKey,
    ascending: bool,
    table: bool,
}

impl Default for View {
    fn default() -> Self {
        View { names: NameMode::Username, key: SortKey::FirstCommit, ascending: true, table: false }
    }
}

/// Height of the Contributors list (grab bag or table, header included), the same on both views so
/// switching between them doesn't resize the window. The Models table is as tall as its rows, up
/// to this.
const LIST_HEIGHT: f32 = 320.0;
const TABLE_HEADER_H: f32 = 24.0;
const TABLE_ROW_H: f32 = 22.0;
/// Room for the sort arrow in every header, so the columns don't move when the sort does.
const SORT_ARROW_W: f32 = 12.0;
/// Padding inside each cell, on each side.
const CELL_PAD: f32 = 6.0;
const NAME_MIN_W: f32 = 110.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CreditsAction {
    #[cfg(test)]
    None,
    #[cfg(not(test))]
    Loading,
    #[cfg(not(test))]
    Refresh,
    Retry,
}

/// Keep the list controls on one line; a small action at the far edge leaves room for all modes.
fn contributor_controls(ui: &mut egui::Ui, t: &Tokens, v: &mut View, keys: &[SortKey], action: CreditsAction) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Show")).color(t.text_secondary));
        egui::ComboBox::from_id_salt("credits_names").width(105.0).selected_text(v.names.label()).show_ui(ui, |ui| {
            for mode in NameMode::ALL {
                ui.selectable_value(&mut v.names, mode, mode.label());
            }
        });
        ui.separator();
        ui.label(RichText::new(tr("Sort")).color(t.text_secondary));
        egui::ComboBox::from_id_salt("credits_sort").width(95.0).selected_text(v.key.label().0).show_ui(ui, |ui| {
            for &key in keys {
                if ui.selectable_label(v.key == key, key.label().0).clicked() {
                    v.key = key;
                    v.ascending = key.default_ascending();
                }
            }
        });
        if ui.small_button(if v.ascending { "⏶" } else { "⏷" }).on_hover_text(tr("Reverse the order")).clicked() {
            v.ascending = !v.ascending;
        }
        ui.separator();
        if ui.selectable_label(!v.table, tr("Grab bag")).clicked() {
            v.table = false;
        }
        if ui.selectable_label(v.table, tr("Table")).clicked() {
            v.table = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match action {
            #[cfg(test)]
            CreditsAction::None => {}
            #[cfg(not(test))]
            CreditsAction::Loading => {
                ui.spinner();
            }
            #[cfg(not(test))]
            CreditsAction::Refresh | CreditsAction::Retry => {
                let hint = if action == CreditsAction::Retry { tr("Retry") } else { tr("Refresh") };
                clicked = ui.small_button("⟳").on_hover_text(hint).clicked();
            }
            #[cfg(test)]
            CreditsAction::Retry => {
                clicked = ui.small_button("⟳").on_hover_text(tr("Retry")).clicked();
            }
        });
    });
    clicked
}

/// About ▸ Contributors: a name toggle, a sort, and the list as a grab bag or a table.
pub fn contributors_ui(ui: &mut egui::Ui) {
    #[cfg(not(test))]
    {
        use crate::credits_github::State;
        match crate::credits_github::state(ui.ctx()) {
            State::Ready(snapshot) => {
                live_contributors_in(ui, &snapshot);
            }
            State::Idle | State::Loading => {
                contributors_in(ui, CONTRIBUTORS, TOTAL_COMMITS, CreditsAction::Loading);
            }
            State::Failed => {
                contributors_in(ui, CONTRIBUTORS, TOTAL_COMMITS, CreditsAction::Retry);
            }
        }
    }
    #[cfg(test)]
    contributors_in(ui, CONTRIBUTORS, TOTAL_COMMITS, CreditsAction::None);
}

/// The public GitHub endpoints have usernames, commit counts and merged PR authors. They do not
/// expose the snapshot's line and date statistics, so the live table shows only known values.
#[cfg(not(test))]
fn live_contributors_in(ui: &mut egui::Ui, snapshot: &crate::credits_github::Snapshot) {
    use crate::credits_github::Contributor as LiveContributor;

    let t = Tokens::get(ui.ctx());
    let id = egui::Id::new("credits_view");
    let mut v = ui.data_mut(|d| d.get_temp::<View>(id)).unwrap_or_default();
    let keys: &[SortKey] = if snapshot.prs_complete { &[SortKey::Name, SortKey::Prs, SortKey::Commits] } else { &[SortKey::Name, SortKey::Commits] };
    if !keys.contains(&v.key) {
        v.key = SortKey::Commits;
        v.ascending = false;
    }
    if contributor_controls(ui, &t, &mut v, keys, CreditsAction::Refresh) {
        crate::credits_github::refresh(ui.ctx());
    }
    let mut list: Vec<&LiveContributor> = snapshot.contributors.iter().collect();
    list.sort_by(|a, b| {
        let order = match v.key {
            SortKey::Prs => a.prs.cmp(&b.prs),
            SortKey::Commits => a.commits.cmp(&b.commits),
            _ => name_key(&live_name(a, v.names)).cmp(&name_key(&live_name(b, v.names))),
        };
        let order = if v.ascending { order } else { order.reverse() };
        order.then_with(|| name_key(&live_name(a, v.names)).cmp(&name_key(&live_name(b, v.names))))
    });
    let counts =
        trf("Contributors: {people} · Commits: {commits}", &[("people", &group(list.len() as u64)), ("commits", &group(snapshot.total_commits))]);
    ui.label(RichText::new(format!("{counts} · {}", tr("Live data from GitHub"))).small().color(t.text_secondary));
    ui.separator();
    if v.table {
        let names = v.names;
        let widths: Vec<Option<f32>> = keys
            .iter()
            .map(|&key| {
                (key != SortKey::Name).then(|| {
                    let values: Vec<String> = list.iter().map(|c| group(if key == SortKey::Prs { c.prs } else { c.commits })).collect();
                    column_width(ui, key.label().1, values.iter().map(String::as_str))
                })
            })
            .collect();
        credits_table(
            ui,
            "credits_live_table",
            &widths,
            NAME_MIN_W,
            LIST_HEIGHT,
            |mut header| {
                for &key in keys {
                    header.col(|ui| {
                        let (long, short) = key.label();
                        let sort = (v.key == key).then_some(v.ascending);
                        if header_cell(ui, &t, short, key.numeric(), Some(sort), key == *keys.last().unwrap_or(&SortKey::Name))
                            .on_hover_text(long)
                            .clicked()
                        {
                            if v.key == key {
                                v.ascending = !v.ascending;
                            } else {
                                v.key = key;
                                v.ascending = key.default_ascending();
                            }
                        }
                    });
                }
            },
            |body| {
                body.rows(TABLE_ROW_H, list.len(), |mut row| {
                    let Some(c) = list.get(row.index()) else { return };
                    row.col(|ui| {
                        ui.add_space(CELL_PAD);
                        live_link(ui, c, names, snapshot.prs_complete);
                    });
                    for &key in keys.iter().skip(1) {
                        row.col(|ui| {
                            cell(ui, true, &group(if key == SortKey::Prs { c.prs } else { c.commits }), key == *keys.last().unwrap_or(&SortKey::Name))
                        });
                    }
                });
            },
        );
    } else {
        egui::ScrollArea::vertical().id_salt("credits_live_bag").auto_shrink([false, false]).max_height(LIST_HEIGHT).show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(14.0, 8.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.spacing_mut().interact_size.y = 0.0;
            ui.horizontal_wrapped(|ui| {
                for c in &list {
                    live_link(ui, c, v.names, snapshot.prs_complete);
                }
            });
        });
    }
    ui.data_mut(|d| d.insert_temp(id, v));
}

#[cfg(not(test))]
fn live_name(c: &crate::credits_github::Contributor, mode: NameMode) -> String {
    CONTRIBUTORS.iter().find(|known| known.login.eq_ignore_ascii_case(&c.login)).map_or_else(|| format!("@{}", c.login), |known| known.name(mode))
}

#[cfg(not(test))]
fn live_link(ui: &mut egui::Ui, c: &crate::credits_github::Contributor, mode: NameMode, prs_complete: bool) {
    let summary = if prs_complete {
        trf("@{login}: {prs} merged PRs, {commits} commits", &[("login", &c.login), ("prs", &group(c.prs)), ("commits", &group(c.commits))])
    } else {
        trf("@{login}: {commits} commits", &[("login", &c.login), ("commits", &group(c.commits))])
    };
    ui.hyperlink_to(live_name(c, mode), format!("https://github.com/{}", c.login)).on_hover_text(summary);
}

/// [`contributors_ui`] over a given list (tests pass their own).
fn contributors_in(ui: &mut egui::Ui, contributors: &[Contributor], total_commits: u64, action: CreditsAction) {
    let t = Tokens::get(ui.ctx());
    let id = egui::Id::new("credits_view");
    let mut v = ui.data_mut(|d| d.get_temp::<View>(id)).unwrap_or_default();
    if contributor_controls(ui, &t, &mut v, &SortKey::ALL, action) && action == CreditsAction::Retry {
        #[cfg(not(test))]
        crate::credits_github::refresh(ui.ctx());
    }
    let list = sorted(contributors, v.names, v.key, v.ascending);
    let counts = trf("Contributors: {people} · Commits: {commits}", &[("people", &group(list.len() as u64)), ("commits", &group(total_commits))]);
    let counts = if action == CreditsAction::Retry { format!("{counts} · {}", tr("GitHub unavailable")) } else { counts };
    ui.label(RichText::new(counts).small().color(t.text_secondary));
    ui.separator();
    if list.is_empty() {
        ui.label(RichText::new(tr("No contributor data was built into this copy.")).color(t.text_secondary));
    } else if v.table {
        table(ui, &list, &mut v, LIST_HEIGHT);
    } else {
        egui::ScrollArea::vertical().id_salt("credits_bag").auto_shrink([false, false]).max_height(LIST_HEIGHT).show(ui, |ui| {
            // Spacing alone separates the names: a "·" between them started some lines.
            ui.spacing_mut().item_spacing = egui::vec2(14.0, 8.0);
            // Each name is one unit: otherwise egui flows a link that doesn't fit onto the next
            // line as wrapped text, at the font's line height instead of the row spacing.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            // horizontal_wrapped gives its first row the interact height; text rows are shorter,
            // so the first gap came out wider than the rest.
            ui.spacing_mut().interact_size.y = 0.0;
            ui.horizontal_wrapped(|ui| {
                for c in &list {
                    link(ui, c, v.names);
                }
            });
        });
    }
    ui.data_mut(|d| d.insert_temp(id, v));
}

/// Height of a table showing all `rows` (header included), up to [`LIST_HEIGHT`].
fn table_height(ui: &egui::Ui, rows: usize) -> f32 {
    // Each row, and the header, is followed by the row gap.
    let gap = ui.spacing().item_spacing.y;
    let body = rows as f32 * (TABLE_ROW_H + gap) + gap;
    (TABLE_HEADER_H + body).min(LIST_HEIGHT)
}

fn link(ui: &mut egui::Ui, c: &Contributor, names: NameMode) -> egui::Response {
    ui.hyperlink_to(c.name(names), format!("https://github.com/{}", c.login)).on_hover_text(c.summary())
}

fn text_width(ui: &egui::Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), egui::Color32::WHITE).size().x)
}

/// A column's width: its widest cell, or its header plus the sort arrow's room, so no column
/// changes width when the sort or the rows change.
fn column_width<'a>(ui: &egui::Ui, header: &str, cells: impl Iterator<Item = &'a str>) -> f32 {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let widest = cells.map(|c| text_width(ui, c, &body)).fold(0.0, f32::max);
    let head = text_width(ui, header, &semibold(body.size)) + SORT_ARROW_W;
    (widest.max(head) + 2.0 * CELL_PAD).ceil()
}

/// A credits table: columns of `widths`, the `None` one taking what the others leave (at least
/// `flex_min`). The header stays put while the rows scroll, within `height` (header included). Too
/// narrow for every column, it scrolls sideways rather than clip one.
fn credits_table(
    ui: &mut egui::Ui,
    id: &str,
    widths: &[Option<f32>],
    flex_min: f32,
    height: f32,
    header: impl FnOnce(egui_extras::TableRow<'_, '_>),
    body: impl FnOnce(egui_extras::TableBody<'_>),
) {
    // The rows' scroll bar sits beside the columns: leave it room, or it pushes the table past the
    // window and a sideways scroll bar appears under it.
    let gutter = scrollbar_gutter(ui);
    let fixed: f32 = widths.iter().flatten().sum::<f32>() + gutter;
    // The table puts a row gap under its header.
    let body_h = (height - TABLE_HEADER_H - ui.spacing().item_spacing.y).max(TABLE_ROW_H);
    egui::ScrollArea::horizontal().id_salt((id, "h")).auto_shrink([false, true]).show(ui, |ui| {
        let flex = (ui.available_width() - fixed).max(flex_min);
        // No gaps between cells: their padding spaces them, and the header rule runs unbroken.
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut b = egui_extras::TableBuilder::new(ui)
            .id_salt(id)
            .striped(true)
            .resizable(false)
            .auto_shrink([false, false])
            .min_scrolled_height(body_h)
            .max_scroll_height(body_h)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
        for (i, w) in widths.iter().enumerate() {
            b = b.column(match w {
                Some(w) => egui_extras::Column::exact(*w + if i + 1 == widths.len() { gutter } else { 0.0 }),
                None => egui_extras::Column::exact(flex).clip(true),
            });
        }
        b.header(TABLE_HEADER_H, header).body(body);
    });
}

fn scrollbar_gutter(ui: &egui::Ui) -> f32 {
    ui.spacing().scroll.allocated_width().max(16.0) + 4.0
}

/// The width of each Contributors column for `list`; the name column (`None`) takes the rest.
fn contributor_widths(ui: &egui::Ui, list: &[&Contributor]) -> Vec<Option<f32>> {
    SortKey::ALL
        .iter()
        .map(|&k| {
            let cells: Vec<String> = list.iter().map(|c| k.cell(c)).collect();
            (k != SortKey::Name).then(|| column_width(ui, k.label().1, cells.iter().map(String::as_str)))
        })
        .collect()
}

/// The Contributors table; clicking a header sorts by it (again reverses it).
fn table(ui: &mut egui::Ui, list: &[&Contributor], v: &mut View, height: f32) {
    let t = Tokens::get(ui.ctx());
    let names = v.names;
    let widths = contributor_widths(ui, list);
    credits_table(
        ui,
        "credits_table",
        &widths,
        NAME_MIN_W,
        height,
        |mut header| {
            for k in SortKey::ALL {
                header.col(|ui| {
                    let sort = (v.key == k).then_some(v.ascending);
                    let (long, short) = k.label();
                    if header_cell(ui, &t, short, k.numeric(), Some(sort), k == SortKey::LastCommit).on_hover_text(long).clicked() {
                        if v.key == k {
                            v.ascending = !v.ascending;
                        } else {
                            v.key = k;
                            v.ascending = k.default_ascending();
                        }
                    }
                });
            }
        },
        |body| {
            body.rows(TABLE_ROW_H, list.len(), |mut row| {
                let Some(c) = list.get(row.index()) else { return };
                row.col(|ui| {
                    ui.add_space(CELL_PAD);
                    link(ui, c, names);
                });
                for k in SortKey::ALL.into_iter().skip(1) {
                    row.col(|ui| cell(ui, k.numeric(), &k.cell(c), k == SortKey::LastCommit));
                }
            });
        },
    );
}

fn cell(ui: &mut egui::Ui, numeric: bool, text: &str, last: bool) {
    let layout = if numeric { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) };
    ui.with_layout(layout, |ui| {
        ui.add_space(CELL_PAD + if last && numeric { scrollbar_gutter(ui) } else { 0.0 });
        ui.add(egui::Label::new(text).selectable(false));
    });
}

/// One header: its label (on the right over counts, like their numbers) under a rule. A sortable
/// one (`sort` is `Some`) is a button, with the arrow on the label's inner side when it's the sort
/// (`Some(Some(ascending))`), in room every header keeps.
fn header_cell(ui: &mut egui::Ui, t: &Tokens, label: &str, numeric: bool, sort: Option<Option<bool>>, last: bool) -> egui::Response {
    let rect = ui.max_rect();
    let resp = ui.allocate_rect(rect, if sort.is_some() { egui::Sense::click() } else { egui::Sense::hover() });
    let kind = if sort.is_some() { egui::WidgetType::Button } else { egui::WidgetType::Label };
    resp.widget_info(|| egui::WidgetInfo::labeled(kind, true, label));
    let resp = if sort.is_some() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    let current = sort.flatten();
    let color = if sort.is_none() || current.is_some() || resp.hovered() { t.text } else { t.text_secondary };
    let size = egui::TextStyle::Body.resolve(ui.style()).size;
    let galley = ui.painter().layout_no_wrap(label.to_string(), semibold(size), color);
    let mut inner = rect.shrink2(egui::vec2(CELL_PAD, 0.0));
    if last {
        inner.max.x -= scrollbar_gutter(ui);
    }
    let y = rect.center().y - galley.size().y / 2.0;
    let (text_x, arrow_x) = if numeric {
        let x = inner.right() - galley.size().x;
        (x, x - SORT_ARROW_W / 2.0)
    } else {
        (inner.left(), inner.left() + galley.size().x + SORT_ARROW_W / 2.0)
    };
    ui.painter().galley(egui::pos2(text_x, y), galley, color);
    if let Some(ascending) = current {
        let (c, h) = (egui::pos2(arrow_x, rect.center().y), 3.0);
        let pts = if ascending {
            vec![c + egui::vec2(-h, h * 0.6), c + egui::vec2(h, h * 0.6), c + egui::vec2(0.0, -h * 0.9)]
        } else {
            vec![c + egui::vec2(-h, -h * 0.6), c + egui::vec2(h, -h * 0.6), c + egui::vec2(0.0, h * 0.9)]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, t.accent, egui::Stroke::NONE));
    }
    let line = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 1.0), rect.max);
    ui.painter().rect_filled(line, 0.0, t.border);
    resp
}

/// About ▸ Models: AI models credited in Co-Authored-By trailers.
pub fn models_ui(ui: &mut egui::Ui) {
    ui.label(RichText::new(tr("AI models credited in commits")).small().color(Tokens::get(ui.ctx()).text_secondary));
    models_in(ui, MODELS);
}

const MODEL_HEADS: [&str; 3] = [tr_noop("Company"), tr_noop("Model"), tr_noop("Version")];

/// The Models tab only needs the name and version already present in the credits snapshot.
fn model_cells(m: &Model) -> [String; 3] {
    [m.company.to_string(), m.model.to_string(), if m.version.is_empty() { "—".into() } else { m.version.to_string() }]
}

/// The width of each Models column; the model name (`None`) takes the spare width.
fn model_widths(ui: &egui::Ui, rows: &[[String; 3]]) -> Vec<Option<f32>> {
    MODEL_HEADS
        .iter()
        .enumerate()
        .map(|(i, h)| (i != 1).then(|| column_width(ui, tr(h), rows.iter().filter_map(|r| r.get(i)).map(String::as_str))))
        .collect()
}

/// [`models_ui`] over a given list (tests pass their own).
fn models_in(ui: &mut egui::Ui, models: &[Model]) {
    let t = Tokens::get(ui.ctx());
    if models.is_empty() {
        ui.label(RichText::new(tr("No model credits were built into this copy.")).color(t.text_secondary));
        return;
    }
    let rows: Vec<[String; 3]> = models.iter().map(model_cells).collect();
    let widths = model_widths(ui, &rows);
    credits_table(
        ui,
        "credits_models",
        &widths,
        NAME_MIN_W,
        table_height(ui, rows.len()),
        |mut header| {
            for (i, h) in MODEL_HEADS.iter().enumerate() {
                header.col(|ui| {
                    header_cell(ui, &t, tr(h), false, None, i + 1 == MODEL_HEADS.len());
                });
            }
        },
        |body| {
            body.rows(TABLE_ROW_H, rows.len(), |mut row| {
                let Some(cells) = rows.get(row.index()) else { return };
                for (i, text) in cells.iter().enumerate() {
                    row.col(|ui| cell(ui, false, text, i + 1 == cells.len()));
                }
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(
        login: &'static str,
        display: Option<&'static str>,
        real: Option<&'static str>,
        added: u64,
        deleted: u64,
        first: &'static str,
    ) -> Contributor {
        Contributor {
            login,
            display_name: display,
            real_name: real,
            prs: 0,
            commits: 1,
            lines_added: added,
            lines_deleted: deleted,
            binary_added: 0,
            binary_deleted: 0,
            first_commit: first,
            last_commit: first,
        }
    }

    fn people() -> [Contributor; 4] {
        [
            c("zed", None, None, 5, 50, "2026-01-02T00:00:00Z"),
            c("Bob", Some("Ann"), None, 10, 0, "2026-01-01T00:00:00Z"),
            c("alice", None, None, 7, 1, ""),
            c("carol", None, Some("Dana"), 1, 0, "2026-01-03T00:00:00Z"),
        ]
    }

    fn logins(v: &[&Contributor]) -> Vec<&'static str> {
        v.iter().map(|c| c.login).collect()
    }

    #[test]
    fn names_fall_back_to_the_username() {
        let p = c("bob", Some("Bobby"), None, 0, 0, "");
        assert_eq!(p.name(NameMode::Username), "@bob");
        assert_eq!(p.name(NameMode::DisplayName), "Bobby");
        assert_eq!(p.name(NameMode::RealName), "@bob");
    }

    #[test]
    fn alphabetical_ignores_at_and_case() {
        let all = people();
        let v = sorted(&all, NameMode::Username, SortKey::Name, true);
        assert_eq!(logins(&v), ["alice", "Bob", "carol", "zed"]);
        let v = sorted(&all, NameMode::DisplayName, SortKey::Name, true);
        assert_eq!(logins(&v), ["alice", "Bob", "carol", "zed"]); // "Ann" sorts with the @usernames
        let v = sorted(&all, NameMode::RealName, SortKey::Name, false);
        assert_eq!(logins(&v), ["zed", "carol", "Bob", "alice"]); // "Dana" between bob and zed
    }

    #[test]
    fn delta_and_dates() {
        let all = people();
        let v = sorted(&all, NameMode::Username, SortKey::LinesDelta, false);
        assert_eq!(logins(&v), ["Bob", "alice", "carol", "zed"]);
        let v = sorted(&all, NameMode::Username, SortKey::FirstCommit, true);
        assert_eq!(logins(&v), ["Bob", "zed", "carol", "alice"]); // undated last
    }

    #[test]
    fn formatting() {
        assert_eq!(group(1234567), "1,234,567");
        assert_eq!(group(12), "12");
        assert_eq!(signed(-1000), "−1,000");
        assert_eq!(day("2026-10-07T18:45:23Z"), "2026-10-07");
        assert_eq!(day(""), "—");
    }

    #[test]
    fn the_built_in_tables_are_consistent() {
        // The compiled-in tables (from contributors.json, or empty without it) hang together.
        for c in CONTRIBUTORS {
            assert!(!c.login.is_empty());
            assert!(c.first_commit <= c.last_commit || c.last_commit.is_empty());
        }
        // Models can come from a separate, newer list than the built-in contributor statistics.
        let mut seen = std::collections::HashSet::new();
        for m in MODELS {
            assert!(!m.company.is_empty() && !m.model.is_empty(), "{m:?}");
            assert!(seen.insert((m.company, m.model, m.version)), "duplicate model: {m:?}");
        }
    }
}

#[cfg(test)]
#[path = "credits_tests.rs"]
mod ui_tests;
