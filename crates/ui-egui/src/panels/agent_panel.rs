//! The right sidebar's Agent tab: a chat with Claude Code or an OpenAI-compatible endpoint that works
//! on the file. The running lives behind [`AgentHost`], which the desktop app provides; nothing here
//! reaches the network or starts a process.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Key, Label, Layout, Margin, Modifiers, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui,
    pos2, vec2,
};
use figcraft_engine::inspect::{InspectOptions, node_json_with_options};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::FigApp;
use crate::i18n::{tr, trf};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, agent_font, agent_medium};
use crate::{a11y, widgets};

const PAD: i8 = 10;
/// A card of tool calls with more rows than this folds behind a header.
const TOOL_ROWS: usize = 4;
const TOOL_ROW_H: f32 = 26.0;
/// What the consent for Claude Code is kept under.
const CLAUDE_TARGET: &str = "claude-code";
/// Claude Code's own choice of model: the CLI gets no `--model` then.
const DEFAULT_MODEL: &str = "default";
/// What the model is told of each selected layer and its direct children.
const SELECTION_FIELDS: &[&str] = &[
    "id",
    "name",
    "type",
    "x",
    "y",
    "width",
    "height",
    "rotation",
    "visible",
    "locked",
    "opacity",
    "fills",
    "strokes",
    "strokeWeight",
    "effects",
    "cornerRadius",
    "layout",
    "sizing",
    "characters",
    "fontFamily",
    "fontStyle",
    "fontSize",
    "mainComponentName",
    "styles",
];
/// How many selected layers are described, and how much text that may take. It goes with every
/// message, so it stays small: the model can always read more with its tools.
const MAX_SELECTION_DESCRIBED: usize = 20;
const MAX_SELECTION_TEXT: usize = 16 * 1024;
const MAX_SELECTION_IDS: usize = 200;
/// How many names the tab shows.
const SELECTION_NAMES: usize = 3;

/// Which backend a run uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentMode {
    #[default]
    ClaudeCode,
    Endpoint,
}

impl AgentMode {
    const ALL: [AgentMode; 2] = [AgentMode::ClaudeCode, AgentMode::Endpoint];

    fn label(self) -> &'static str {
        match self {
            AgentMode::ClaudeCode => tr("Claude Code"),
            AgentMode::Endpoint => tr("Endpoint"),
        }
    }
}

/// What the tab remembers. API keys are not here: only which URLs have one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    pub mode: AgentMode,
    pub claude_path: String,
    pub claude_model: String,
    pub url: String,
    pub model: String,
    /// The endpoint's model takes images.
    pub vision: bool,
    /// The endpoints with a saved key.
    pub keyed: Vec<String>,
    /// The destinations the person agreed to send file contents to.
    pub allowed: Vec<String>,
}

impl AgentSettings {
    /// Where a run sends the file's contents, as consent is kept.
    fn target(&self) -> &str {
        match self.mode {
            AgentMode::ClaudeCode => CLAUDE_TARGET,
            AgentMode::Endpoint => self.url.trim(),
        }
    }

    fn allowed_now(&self) -> bool {
        self.allowed.iter().any(|a| a == self.target())
    }

    /// Who is asked for models: the `claude` program or the endpoint's URL.
    pub fn source(&self) -> &str {
        match self.mode {
            AgentMode::ClaudeCode => self.claude_path.trim(),
            AgentMode::Endpoint => self.url.trim(),
        }
    }

    fn model_mut(&mut self) -> &mut String {
        match self.mode {
            AgentMode::ClaudeCode => &mut self.claude_model,
            AgentMode::Endpoint => &mut self.model,
        }
    }

    /// The model picked, as this mode's program or endpoint takes it (empty: its default).
    pub fn model_name(&self) -> &str {
        match self.mode {
            AgentMode::ClaudeCode => Some(self.claude_model.trim()).filter(|m| *m != DEFAULT_MODEL).unwrap_or_default(),
            AgentMode::Endpoint => self.model.trim(),
        }
    }

    /// Everything a run needs, apart from a prompt.
    fn ready(&self) -> bool {
        self.allowed_now() && !self.source().is_empty() && (self.mode == AgentMode::ClaudeCode || !self.model_name().is_empty())
    }
}

/// A model to choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentModel {
    /// The name the program or endpoint takes.
    pub value: String,
    pub label: String,
    pub detail: String,
}

/// One message to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRun {
    pub settings: AgentSettings,
    /// What the person typed, as the chat shows it.
    pub prompt: String,
    /// What the model is told of the selection when it was sent ([`selection_context`]).
    pub context: String,
    /// The file it is for (a `doc.list` id): the run stays on it.
    pub document: Option<u64>,
}

impl AgentRun {
    /// What the model gets: the selection travels with the prompt.
    pub fn message(&self) -> String {
        let prompt = self.prompt.trim();
        if self.context.is_empty() { prompt.to_string() } else { format!("{prompt}\n\n{}", self.context) }
    }
}

/// The selection as the model reads it, so that "this" and "make it red" mean something: what each
/// layer is and looks like, its direct children, and every id for the tools. A big selection shortens
/// to names, then to ids, to stay within [`MAX_SELECTION_TEXT`].
pub(crate) fn selection_context(doc: &figcraft_doc::Document, ids: &[figcraft_doc::NodeId]) -> String {
    if ids.is_empty() {
        return "<selection>Nothing is selected.</selection>".to_string();
    }
    let parsed = |p: serde_json::Value| InspectOptions::parse(&p).unwrap_or_default();
    let (full, brief) = (parsed(json!({"fields": SELECTION_FIELDS})), parsed(json!({"outline": true})));
    let (mut described, mut used) = (Vec::new(), 0);
    for &id in ids.iter().take(MAX_SELECTION_DESCRIBED) {
        let text = Some(node_json_with_options(doc, id, 1, &full).to_string()).filter(|t| used + t.len() <= MAX_SELECTION_TEXT);
        let Some(text) =
            text.or_else(|| Some(node_json_with_options(doc, id, 0, &brief).to_string()).filter(|t| used + t.len() <= MAX_SELECTION_TEXT))
        else {
            break;
        };
        used += text.len();
        // A layer's text can't close the block and pass for what the app says: JSON reads this the same.
        described.push(text.replace('<', "\\u003c"));
    }
    let listed: Vec<String> = ids.iter().take(MAX_SELECTION_IDS).map(|id| id.0.to_string()).collect();
    let mut block = format!("<selection count=\"{}\" ids=\"{}\">", ids.len(), listed.join(","));
    for text in &described {
        block.push('\n');
        block.push_str(text);
    }
    if described.len() < ids.len() {
        block.push_str(&format!("\n({} more layers aren't described here: use get_node)", ids.len() - described.len()));
    }
    block.push_str("\n</selection>");
    block
}

/// What the tab says is selected: the first few names.
fn selection_caption(doc: Option<&figcraft_doc::Document>, ids: &[figcraft_doc::NodeId]) -> String {
    let names: Vec<&str> = ids.iter().take(SELECTION_NAMES).filter_map(|id| doc?.node(*id).map(|n| n.name.as_str())).collect();
    if names.is_empty() {
        return tr("Nothing selected").to_string();
    }
    let more = ids.len().saturating_sub(names.len());
    let layers = if more > 0 { format!("{} +{more}", names.join(", ")) } else { names.join(", ") };
    trf("Selected: {layers}", &[("layers", &layers)])
}

/// What a line of the chat is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentLineKind {
    Prompt,
    Text,
    Tool,
    Error,
    Done,
}

/// One line of the chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentLine {
    pub kind: AgentLineKind,
    /// The words; of a tool call, the tool's name.
    pub text: String,
    /// What a tool call is for: may be empty.
    pub detail: String,
}

/// Calls in a row of one tool for one thing, shown as one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ToolRow<'a> {
    pub name: &'a str,
    pub detail: &'a str,
    pub count: usize,
}

/// What the chat draws: a line, or a card of the tool calls in a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Block<'a> {
    Line(&'a AgentLine),
    Tools(Vec<ToolRow<'a>>),
}

pub(crate) fn blocks(lines: &[AgentLine]) -> Vec<Block<'_>> {
    let mut out = Vec::new();
    for line in lines {
        if line.kind != AgentLineKind::Tool {
            out.push(Block::Line(line));
            continue;
        }
        let row = ToolRow { name: &line.text, detail: &line.detail, count: 1 };
        match out.last_mut() {
            Some(Block::Tools(rows)) => match rows.last_mut() {
                Some(last) if last.name == row.name && last.detail == row.detail => last.count += 1,
                _ => rows.push(row),
            },
            _ => out.push(Block::Tools(vec![row])),
        }
    }
    out
}

/// What a person reads for a tool's name, and its icon.
pub(crate) fn tool_look(name: &str) -> (String, Icon) {
    let (label, icon) = match name {
        "list_commands" => (tr("Find commands"), Icon::Search),
        "run_command" => (tr("Run command"), Icon::Play),
        "batch" => (tr("Run commands"), Icon::Actions),
        "create_node" => (tr("Create layer"), Icon::Plus),
        "set_properties" => (tr("Change properties"), Icon::Adjust),
        "inspect_document" => (tr("Read document"), Icon::Layers),
        "get_node" => (tr("Read layer"), Icon::Eye),
        "select" => (tr("Select layers"), Icon::Move),
        "screenshot" => (tr("Take screenshot"), Icon::Image),
        "undo" => (tr("Undo"), Icon::Undo),
        "redo" => (tr("Redo"), Icon::Redo),
        // A tool that isn't in the app's list: its own name, readable.
        other => {
            let words = other.replace('_', " ");
            let mut chars = words.chars();
            let first = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
            return (first + chars.as_str(), Icon::Code);
        }
    };
    (label.to_string(), icon)
}

/// The tokens a chat has used so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgentUsage {
    pub input: u64,
    pub output: u64,
}

/// Runs the tab's models (the desktop app; the browser has none).
pub trait AgentHost {
    fn settings(&self) -> AgentSettings;
    fn save_settings(&mut self, settings: &AgentSettings);
    /// Keep `key` for `url` where only the person can read it.
    fn set_key(&mut self, url: &str, key: &str) -> Result<(), String>;
    /// Send a message in the background; the model remembers the chat so far.
    fn start(&mut self, run: AgentRun) -> Result<(), String>;
    fn stop(&mut self);
    fn running(&self) -> bool;
    fn lines(&self) -> Vec<AgentLine>;
    fn usage(&self) -> AgentUsage;
    /// Start a new chat: the lines, what the model remembers and the token counts go.
    fn clear(&mut self);
    /// Ask this mode's program or endpoint for its models in the background.
    fn load_models(&mut self, settings: &AgentSettings);
    fn models(&self, settings: &AgentSettings) -> Vec<AgentModel>;
    /// Where a `claude` program is on this computer.
    fn detect_claude(&self) -> Option<String>;
}

/// What the tab holds between frames.
#[derive(Default)]
pub struct AgentUi {
    settings: Option<AgentSettings>,
    prompt: String,
    /// A key being typed: gone once saved.
    key: String,
    note: Option<String>,
    show_settings: bool,
    /// The program or endpoint already asked for its models.
    models_from: Option<String>,
    /// A field that names one has the focus: it isn't finished yet.
    typing_source: bool,
}

/// Whether a message can be sent.
pub(crate) fn can_run(settings: &AgentSettings, prompt: &str, running: bool) -> bool {
    !running && settings.ready() && !prompt.trim().is_empty()
}

/// `1234567` as `1,234,567`.
pub(crate) fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn caption(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.label(RichText::new(text).color(t.text_secondary).font(agent_font(11.0)));
}

fn field(ui: &mut Ui, t: &Tokens, label: &str, text: &mut String, hint: &str) -> bool {
    caption(ui, t, label);
    ui.add(TextEdit::singleline(text).hint_text(hint).font(agent_font(12.0)).desired_width(ui.available_width())).has_focus()
}

/// What the tab's buttons asked for: done once it has drawn.
#[derive(Default)]
struct Asked {
    detect: bool,
    load_models: bool,
    save_key: bool,
    run: bool,
    stop: bool,
    clear: bool,
}

/// The consent to send the file to this destination.
fn consent(ui: &mut Ui, settings: &mut AgentSettings) {
    let destination = match settings.mode {
        AgentMode::ClaudeCode => "Anthropic".to_string(),
        AgentMode::Endpoint => settings.url.trim().to_string(),
    };
    let on = settings.allowed_now();
    if widgets::checkbox(ui, on, &trf("Send this file's contents to {target}", &[("target", &destination)])) {
        let key = settings.target().to_string();
        if on {
            settings.allowed.retain(|a| *a != key);
        } else if !key.is_empty() {
            settings.allowed.push(key);
        }
    }
}

fn settings_section(ui: &mut Ui, t: &Tokens, state: &mut AgentUi, settings: &mut AgentSettings, asked: &mut Asked) {
    match settings.mode {
        AgentMode::ClaudeCode => {
            state.typing_source = field(ui, t, tr("Claude Code program"), &mut settings.claude_path, "claude");
            asked.detect = widgets::button(ui, tr("Detect"), false, true).clicked();
        }
        AgentMode::Endpoint => {
            state.typing_source = field(ui, t, tr("OpenAI-compatible URL"), &mut settings.url, "http://localhost:11434/v1");
            caption(ui, t, tr("API key"));
            let saved = settings.keyed.iter().any(|k| k == settings.url.trim());
            let hint = if saved { tr("A key is saved for this URL") } else { tr("None needed for most local servers") };
            ui.add(TextEdit::singleline(&mut state.key).password(true).hint_text(hint).font(agent_font(12.0)).desired_width(ui.available_width()));
            asked.save_key = widgets::button(ui, tr("Save key"), false, !state.key.is_empty() && !settings.url.trim().is_empty()).clicked();
            if widgets::checkbox(ui, settings.vision, tr("The model can see images")) {
                settings.vision = !settings.vision;
            }
        }
    }
    field(ui, t, tr("Other model"), settings.model_mut(), tr("Any name it takes"));
    asked.load_models = widgets::button(ui, tr("Refresh models"), false, !settings.source().is_empty()).clicked();
    consent(ui, settings);
    ui.add_space(6.0);
    ui.separator();
}

/// An icon in a space of its own in the row.
fn icon(ui: &mut Ui, size: f32, icon: Icon, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    icons::paint(ui.painter(), rect, icon, color);
}

fn tool_header(ui: &mut Ui, t: &Tokens, calls: usize, open: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), TOOL_ROW_H), Sense::click());
    let label = trf("{n} tool calls", &[("n", &calls)]);
    a11y::button(&resp, &label);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.hover);
    }
    let mark = Rect::from_center_size(pos2(rect.min.x + 10.0, rect.center().y), vec2(14.0, 14.0));
    icons::paint(ui.painter(), mark, if open { Icon::Chevron } else { Icon::ChevronRight }, t.icon_secondary);
    ui.painter().text(pos2(rect.min.x + 26.0, rect.center().y), Align2::LEFT_CENTER, label, agent_medium(12.0), t.text_secondary);
    resp.clicked()
}

fn tool_row(ui: &mut Ui, t: &Tokens, row: &ToolRow<'_>, active: bool) {
    let (label, mark) = tool_look(row.name);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), TOOL_ROW_H), Sense::hover());
    let said = [label.as_str(), row.detail].join(" ");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, said.trim_end()));
    let middle = rect.center().y;
    let tile = Rect::from_center_size(pos2(rect.min.x + 10.0, middle), vec2(20.0, 20.0));
    ui.painter().rect_filled(tile, CornerRadius::same(5), t.hover);
    icons::paint(ui.painter(), tile.shrink(4.0), mark, if active { t.accent_text } else { t.icon_secondary });

    let mut right = rect.max.x;
    if active {
        let at = Rect::from_center_size(pos2(right - 7.0, middle), vec2(14.0, 14.0));
        egui::Spinner::new().size(12.0).paint_at(ui, at);
        right -= 20.0;
    }
    if row.count > 1 {
        let times = ui.painter().layout_no_wrap(format!("×{}", row.count), agent_medium(11.0), t.text_secondary);
        right -= times.size().x;
        ui.painter().galley(pos2(right, middle - times.size().y / 2.0), times, t.text_secondary);
        right -= 6.0;
    }
    let left = rect.min.x + 26.0;
    let name = ui.painter().layout_no_wrap(label, agent_medium(12.0), t.text);
    let name_w = name.size().x;
    ui.painter().galley(pos2(left, middle - name.size().y / 2.0), name, t.text);
    let room = right - left - name_w - 6.0;
    if row.detail.is_empty() || room < 16.0 {
        return;
    }
    let mut job = LayoutJob::single_section(row.detail.to_string(), TextFormat::simple(agent_font(11.0), t.text_secondary));
    job.wrap = TextWrapping::truncate_at_width(room);
    let note = ui.painter().layout_job(job);
    let cut = note.elided;
    ui.painter().galley(pos2(left + name_w + 6.0, middle - note.size().y / 2.0), note, t.text_secondary);
    if cut {
        resp.on_hover_text(RichText::new(row.detail).font(agent_font(11.0)));
    }
}

/// The tool calls in a row: `at` tells the cards apart and `active` is the one still going.
fn tool_card(ui: &mut Ui, t: &Tokens, at: usize, rows: &[ToolRow<'_>], active: bool) {
    let id = ui.make_persistent_id(("agent_tools", at));
    let open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let folds = rows.len() > TOOL_ROWS;
    ui.add_space(4.0);
    Frame::NONE.fill(t.field).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(8, 4)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            if folds && tool_header(ui, t, rows.iter().map(|r| r.count).sum(), open) {
                ui.data_mut(|d| d.insert_temp(id, !open));
            }
            // Folded, a card still going shows its latest calls; a finished one shows only its header.
            let shown = match (folds && !open, active) {
                (false, _) => rows,
                (true, true) => rows.get(rows.len().saturating_sub(TOOL_ROWS)..).unwrap_or(rows),
                (true, false) => &[],
            };
            for (i, row) in shown.iter().enumerate() {
                tool_row(ui, t, row, active && i + 1 == shown.len());
            }
        },
    );
    ui.add_space(4.0);
}

fn chat_line(ui: &mut Ui, t: &Tokens, line: &AgentLine) {
    match line.kind {
        AgentLineKind::Prompt => {
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                let widest = ui.available_width() * 0.85;
                Frame::NONE.fill(t.selected_row).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(10, 7)).show(ui, |ui| {
                    ui.set_max_width(widest);
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.add(Label::new(RichText::new(&line.text).color(t.text).font(agent_font(13.0)).line_height(Some(18.0))).wrap());
                    });
                });
            });
            ui.add_space(6.0);
        }
        AgentLineKind::Text => {
            ui.add_space(2.0);
            ui.add(Label::new(RichText::new(&line.text).color(t.text).font(agent_font(13.0)).line_height(Some(18.0))).wrap());
            ui.add_space(4.0);
        }
        AgentLineKind::Error => {
            ui.add_space(4.0);
            Frame::NONE.fill(t.danger.gamma_multiply(0.12)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    icon(ui, 14.0, Icon::GamutWarning, t.danger);
                    ui.add(Label::new(RichText::new(&line.text).color(t.danger).font(agent_font(12.0))).wrap());
                });
            });
            ui.add_space(4.0);
        }
        AgentLineKind::Done => {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                icon(ui, 12.0, Icon::Check, t.success);
                ui.label(RichText::new(&line.text).color(t.text_secondary).font(agent_font(11.0)));
            });
            ui.add_space(6.0);
        }
        // Tool calls go in cards ([`blocks`]).
        AgentLineKind::Tool => {}
    }
}

fn chat(ui: &mut Ui, t: &Tokens, lines: &[AgentLine], running: bool) {
    if lines.is_empty() {
        ui.add(
            Label::new(RichText::new(tr("Ask the agent to design or change something in this file.")).color(t.text_secondary).font(agent_font(13.0)))
                .wrap(),
        );
    }
    let blocks = blocks(lines);
    let last = blocks.len().saturating_sub(1);
    for (at, block) in blocks.iter().enumerate() {
        match block {
            Block::Tools(rows) => tool_card(ui, t, at, rows, running && at == last),
            Block::Line(line) => chat_line(ui, t, line),
        }
    }
}

/// Which backend the tab uses, as one control.
fn mode_tabs(ui: &mut Ui, t: &Tokens, mode: &mut AgentMode) {
    Frame::NONE.fill(t.field).corner_radius(CornerRadius::same(7)).inner_margin(Margin::same(2)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let width = ((ui.available_width() - 2.0) / AgentMode::ALL.len() as f32).max(40.0);
        ui.horizontal(|ui| {
            for tab in AgentMode::ALL {
                let (label, active) = (tab.label(), *mode == tab);
                let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
                if active || resp.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(5), if active { t.panel } else { t.hover });
                }
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, agent_medium(12.0), if active { t.text } else { t.text_secondary });
                if resp.clicked() {
                    *mode = tab;
                }
            }
        });
    });
}

/// The model's name to show for what's picked.
fn picked_label(settings: &AgentSettings, models: &[AgentModel]) -> String {
    let picked = match settings.mode {
        AgentMode::ClaudeCode => settings.claude_model.trim(),
        AgentMode::Endpoint => settings.model.trim(),
    };
    let wanted = if picked.is_empty() && settings.mode == AgentMode::ClaudeCode { DEFAULT_MODEL } else { picked };
    models.iter().find(|m| m.value == wanted).map_or_else(|| wanted.to_string(), |m| m.label.clone())
}

#[allow(clippy::too_many_arguments)]
fn input(
    ui: &mut Ui,
    t: &Tokens,
    state: &mut AgentUi,
    settings: &mut AgentSettings,
    running: bool,
    models: &[AgentModel],
    selected: &str,
    asked: &mut Asked,
) {
    if let Some(note) = &state.note {
        ui.add(Label::new(RichText::new(note).color(t.danger).font(agent_font(11.0))).wrap());
    }
    ui.add(Label::new(RichText::new(selected).color(t.text_secondary).font(agent_font(11.0))).truncate());
    if !settings.allowed_now() && !settings.source().is_empty() {
        consent(ui, settings);
    }
    let id = ui.make_persistent_id("agent_prompt");
    // Enter sends and Shift+Enter starts a new line, taken before the field sees them.
    if ui.memory(|m| m.has_focus(id)) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        asked.run = true;
    }
    ui.add_space(4.0);
    let edge = if ui.memory(|m| m.has_focus(id)) { t.accent } else { t.border };
    Frame::NONE.fill(t.field).stroke(Stroke::new(1.0, edge)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(8)).show(ui, |ui| {
        ui.add(
            TextEdit::multiline(&mut state.prompt)
                .id(id)
                .hint_text(tr("Message the agent"))
                .font(agent_font(13.0))
                .frame(Frame::NONE)
                .desired_rows(3)
                .desired_width(ui.available_width()),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let button = 56.0;
            let shown = picked_label(settings, models);
            let width = (ui.available_width() - button - 8.0).max(60.0);
            craft_ui::ComboBox::from_id_salt("agent_model").selected_text(RichText::new(shown).font(agent_font(12.0))).width(width).show_ui(
                ui,
                |ui| {
                    for model in models {
                        let on = picked_label(settings, std::slice::from_ref(model)) == model.label;
                        let row = ui.selectable_label(on, RichText::new(&model.label).font(agent_font(12.0)));
                        let row = if model.detail.is_empty() { row } else { row.on_hover_text(RichText::new(&model.detail).font(agent_font(11.0))) };
                        if row.clicked() {
                            *settings.model_mut() = model.value.clone();
                        }
                    }
                },
            );
            if running {
                asked.stop = widgets::button(ui, tr("Stop"), false, true).clicked();
            } else if widgets::button(ui, tr("Send"), true, can_run(settings, &state.prompt, running)).clicked() {
                asked.run = true;
            }
        });
    });
}

/// The Agent tab's contents, filling the panel below the tab row.
pub fn panel(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let selection = app.session.selection();
    let document = app.session.ds().ok().map(|d| d.uid);
    let selected = selection_caption(app.session.doc().ok(), &selection);
    let Some(host) = app.services.agent() else {
        ui.add_space(f32::from(PAD));
        ui.horizontal(|ui| {
            ui.add_space(f32::from(PAD));
            ui.add(Label::new(RichText::new(tr("The agent needs the desktop app.")).color(t.text_secondary).font(agent_font(12.0))).wrap());
        });
        return;
    };
    let state = &mut app.ui.agent;
    let mut settings = state.settings.take().unwrap_or_else(|| {
        let mut saved = host.settings();
        if saved.claude_path.is_empty() {
            saved.claude_path = host.detect_claude().unwrap_or_default();
        }
        saved
    });
    let before = settings.clone();
    // The models come from what's named, once it's finished being typed.
    let source = settings.source().to_string();
    if !source.is_empty() && !state.typing_source && state.models_from.as_deref() != Some(source.as_str()) {
        host.load_models(&settings);
        state.models_from = Some(source);
    }
    let (running, models, lines, usage) = (host.running(), host.models(&settings), host.lines(), host.usage());
    let mut asked = Asked::default();
    egui::Panel::bottom("agent_input").resizable(false).frame(Frame::NONE.fill(t.panel).inner_margin(Margin::same(PAD))).show(ui, |ui| {
        ui.style_mut().override_font_id = Some(agent_font(12.0));
        input(ui, &t, state, &mut settings, running, &models, &selected, &mut asked);
    });
    egui::CentralPanel::default().frame(Frame::NONE.fill(t.panel).inner_margin(Margin::symmetric(PAD, 6))).show(ui, |ui| {
        ui.style_mut().override_font_id = Some(agent_font(12.0));
        mode_tabs(ui, &t, &mut settings.mode);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            asked.clear = widgets::button(ui, tr("New chat"), false, !running && !lines.is_empty()).clicked();
            if widgets::button(ui, tr("Settings"), false, true).clicked() {
                state.show_settings = !state.show_settings;
            }
            if running {
                ui.spinner();
            }
        });
        let (input_tokens, output_tokens) = (grouped(usage.input), grouped(usage.output));
        caption(ui, &t, &trf("Tokens: {input} in, {output} out", &[("input", &input_tokens), ("output", &output_tokens)]));
        ui.add_space(4.0);
        ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            if state.show_settings || settings.source().is_empty() {
                settings_section(ui, &t, state, &mut settings, &mut asked);
            } else {
                state.typing_source = false;
            }
            chat(ui, &t, &lines, running);
        });
    });
    if asked.detect {
        match host.detect_claude() {
            Some(path) => settings.claude_path = path,
            None => state.note = Some(tr("No claude program found: enter its path.").to_string()),
        }
    }
    if asked.load_models {
        host.load_models(&settings);
        state.models_from = Some(settings.source().to_string());
    }
    if asked.save_key {
        let url = settings.url.trim().to_string();
        match host.set_key(&url, &state.key) {
            Ok(()) => {
                state.key.clear();
                state.note = None;
                if !settings.keyed.contains(&url) {
                    settings.keyed.push(url);
                }
                // A key can open up the list of models.
                host.load_models(&settings);
            }
            Err(why) => state.note = Some(why),
        }
    }
    if asked.stop {
        host.stop();
    }
    if asked.clear {
        host.clear();
    }
    if settings != before {
        host.save_settings(&settings);
    }
    if asked.run && can_run(&settings, &state.prompt, running) {
        // As it is now: the person may have selected something else since the last message.
        let context = app.session.doc().map(|doc| selection_context(doc, &selection)).unwrap_or_default();
        let run = AgentRun { settings: settings.clone(), prompt: state.prompt.trim().to_string(), context, document };
        match host.start(run) {
            Ok(()) => {
                state.prompt.clear();
                state.note = None;
            }
            Err(why) => state.note = Some(why),
        }
    }
    state.settings = Some(settings);
}
