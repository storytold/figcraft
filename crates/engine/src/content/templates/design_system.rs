//! Design system starter: colour and text styles, a Theme variable collection with Light and Dark
//! modes and a Sizes collection, shown on a Foundations page; and a Components page with button and
//! input component sets and a card, all bound to the variables, used in one form shown in both
//! modes.

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use super::kit::{H, Kit, TYPE, V, merged};
use crate::Result;

const THEME: &str = "Theme";
const MODES: [&str; 2] = ["Light", "Dark"];

/// Theme colours: (name, Light, Dark).
const THEME_COLORS: [(&str, &str, &str); 9] = [
    ("Surface", "#FFFFFF", "#121816"),
    ("Surface raised", "#F5F7F6", "#1C2421"),
    ("Text", "#14201C", "#F2F5F4"),
    ("Text muted", "#5C6B66", "#9FADA8"),
    ("Primary", "#1F6F5C", "#58C2A6"),
    ("Primary hover", "#185848", "#7AD3BA"),
    ("On primary", "#FFFFFF", "#0C1A16"),
    ("Border", "#D8DFDC", "#34403B"),
    ("Danger", "#B4472E", "#F08A72"),
];

/// Sizes: (name, value).
const SIZES: [(&str, f64); 7] =
    [("Radius/S", 6.0), ("Radius/M", 12.0), ("Radius/L", 24.0), ("Space/S", 8.0), ("Space/M", 16.0), ("Space/L", 24.0), ("Space/XL", 40.0)];

/// Text in a text style whose colour follows theme variable `var`.
fn themed(k: &mut Kit, parent: NodeId, chars: &str, style: &str, var: &str, props: Value) -> Result<NodeId> {
    let id = k.text_with(parent, chars, style, "#14201C", props)?;
    k.bound(id, &[("fill", var)])
}

/// A surface whose fill, border and corner radius follow the variables.
fn surface(k: &mut Kit, kind: &str, parent: NodeId, name: &str, dir: &str, [gap, pad]: [f64; 2], fill: &str, props: Value) -> Result<NodeId> {
    let base = json!({"fill": "#FFFFFF", "stroke": "#D8DFDC", "strokeWeight": 1});
    let id = k.stack_of(kind, parent, name, dir, gap, json!(pad), merged(base, props))?;
    k.bound(id, &[("fill", fill), ("stroke", "Border"), ("cornerRadius", "Radius/L")])
}

pub(super) fn build(k: &mut Kit) -> Result<()> {
    k.foundations()?;
    let colors = THEME_COLORS.iter().map(|(name, light, dark)| (*name, "color", vec![json!(light), json!(dark)])).collect();
    k.variables(THEME, &MODES, colors)?;
    k.variables("Sizes", &["Default"], SIZES.iter().map(|(name, v)| (*name, "number", vec![json!(v)])).collect())?;
    k.rename_page("Foundations")?;
    let page = k.page()?;
    for (i, mode) in MODES.into_iter().enumerate() {
        colours(k, page, mode, i as f64 * 600.0)?;
    }
    type_scale(k, page, 1200.0)?;
    sizes_board(k, page, 1800.0)?;
    let page = k.add_page("Components")?;
    let button = buttons(k, page)?;
    let input = inputs(k, page)?;
    card(k, page, button)?;
    for (i, mode) in MODES.into_iter().enumerate() {
        example(k, page, mode, 1120.0 + i as f64 * 480.0, button, input)?;
    }
    Ok(())
}

/// A board on the canvas: a titled panel that follows the theme.
fn board(k: &mut Kit, page: NodeId, title: &str, note: &str, x: f64, width: f64) -> Result<NodeId> {
    let board = surface(k, "frame", page, title, V, [24.0, 32.0], "Surface", json!({"sizingH": "fixed", "width": width, "x": x}))?;
    let head = k.stack(board, "Heading", V, 4.0, json!(0), json!({"sizingH": "fill"}))?;
    themed(k, head, title, "Heading/M", "Text", json!({}))?;
    themed(k, head, note, "Body/S", "Text muted", json!({}))?;
    Ok(board)
}

fn colours(k: &mut Kit, page: NodeId, mode: &str, x: f64) -> Result<()> {
    let board = board(k, page, &format!("Colours · {mode}"), "The Theme collection's colour variables in this mode", x, 520.0)?;
    k.mode(board, THEME, mode)?;
    let grid = k.stack(board, "Swatches", H, 16.0, json!(0), json!({"sizingH": "fill", "wrap": true, "crossGap": 20}))?;
    for (name, ..) in THEME_COLORS {
        let swatch = k.stack(grid, name, V, 8.0, json!(0), json!({}))?;
        let chip = k.rect(
            swatch,
            "Colour",
            [0.0, 0.0, 102.0, 64.0],
            json!({"fill": "#FFFFFF", "stroke": "#D8DFDC", "strokeWeight": 1, "cornerRadius": 12}),
        )?;
        k.bound(chip, &[("fill", name), ("stroke", "Border"), ("cornerRadius", "Radius/M")])?;
        themed(k, swatch, name, "Label/S", "Text", json!({}))?;
    }
    Ok(())
}

fn type_scale(k: &mut Kit, page: NodeId, x: f64) -> Result<()> {
    let board = board(k, page, "Type", "The text styles, largest first", x, 520.0)?;
    for (name, size, weight, line) in TYPE {
        let row = k.stack(board, name, V, 2.0, json!(0), json!({"sizingH": "fill"}))?;
        themed(k, row, &format!("{name} · {weight} {size} / {line}"), "Label/S", "Text muted", json!({}))?;
        themed(k, row, "Ridgeline at first light", name, "Text", json!({"sizingH": "fill", "textAutoResize": "height"}))?;
    }
    Ok(())
}

fn sizes_board(k: &mut Kit, page: NodeId, x: f64) -> Result<()> {
    let board = board(k, page, "Sizes", "Number variables for corner radius and spacing", x, 400.0)?;
    let radii = k.stack(board, "Radius", H, 16.0, json!(0), json!({"sizingH": "fill"}))?;
    let spaces = k.stack(board, "Space", V, 12.0, json!(0), json!({"sizingH": "fill"}))?;
    for (name, value) in SIZES {
        if name.starts_with("Radius") {
            let item = k.stack(radii, name, V, 8.0, json!(0), json!({}))?;
            let square = k.rect(item, "Corner", [0.0, 0.0, 72.0, 72.0], json!({"fill": "#F5F7F6", "stroke": "#D8DFDC", "strokeWeight": 1}))?;
            k.bound(square, &[("fill", "Surface raised"), ("stroke", "Border"), ("cornerRadius", name)])?;
            themed(k, item, &format!("{name} · {value}"), "Label/S", "Text", json!({}))?;
        } else {
            let item = k.stack(spaces, name, H, 12.0, json!(0), json!({"crossAlign": "center"}))?;
            let bar = k.rect(item, "Space", [0.0, 0.0, value, 16.0], json!({"fill": "#1F6F5C", "cornerRadius": 3}))?;
            k.bound(bar, &[("fill", "Primary"), ("width", name)])?;
            themed(k, item, &format!("{name} · {value}"), "Label/S", "Text", json!({}))?;
        }
    }
    Ok(())
}

/// The button component set: Type × State, with a Label text property.
fn buttons(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let mut variants = Vec::new();
    let mut labels = Vec::new();
    for (row, kind) in ["Primary", "Secondary", "Ghost"].into_iter().enumerate() {
        for (column, state) in ["Default", "Hover", "Disabled"].into_iter().enumerate() {
            let hover = state == "Hover";
            // (fill, label colour, outlined)
            let (fill, ink, outlined) = match kind {
                "Primary" => (Some(if hover { "Primary hover" } else { "Primary" }), "On primary", false),
                "Secondary" => (Some(if hover { "Surface raised" } else { "Surface" }), "Text", true),
                _ => (hover.then_some("Surface raised"), "Primary", false),
            };
            let mut props = json!({"mainAlign": "center", "crossAlign": "center", "x": column as f64 * 150.0, "y": row as f64 * 68.0});
            if fill.is_some() {
                props = merged(props, json!({"fill": "#1F6F5C"}));
            }
            if outlined {
                props = merged(props, json!({"stroke": "#D8DFDC", "strokeWeight": 1}));
            }
            if state == "Disabled" {
                props = merged(props, json!({"opacity": 0.4}));
            }
            let c = k.stack_of("component", page, &format!("Type={kind}, State={state}"), H, 8.0, json!([11, 20, 11, 20]), props)?;
            k.bound(c, &[("cornerRadius", "Radius/M")])?;
            if let Some(fill) = fill {
                k.bound(c, &[("fill", fill)])?;
            }
            if outlined {
                k.bound(c, &[("stroke", "Border")])?;
            }
            labels.push(themed(k, c, "Button", "Label/M", ink, json!({}))?);
            variants.push(c);
        }
    }
    let set = k.variants("Button", &variants, json!({}))?;
    k.text_prop(set, "Label", "Button", &labels)?;
    Ok(set)
}

/// The input component set: State, with Label and Value text properties.
fn inputs(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let mut variants = Vec::new();
    let (mut labels, mut values) = (Vec::new(), Vec::new());
    for (i, state) in ["Default", "Focus", "Error"].into_iter().enumerate() {
        let (edge, weight, help, help_ink) = match state {
            "Focus" => ("Primary", 2, "Helper text", "Text muted"),
            "Error" => ("Danger", 1, "This field is required", "Danger"),
            _ => ("Border", 1, "Helper text", "Text muted"),
        };
        let props = json!({"sizingH": "fixed", "width": 300, "x": i as f64 * 340.0, "y": 300});
        let c = k.stack_of("component", page, &format!("State={state}"), V, 6.0, json!(0), props)?;
        let label = themed(k, c, "Label", "Label/S", "Text", json!({}))?;
        let field = json!({"fill": "#FFFFFF", "stroke": "#D8DFDC", "strokeWeight": weight, "sizingH": "fill", "crossAlign": "center"});
        let field = k.stack(c, "Field", H, 8.0, json!([11, 14, 11, 14]), field)?;
        k.bound(field, &[("fill", "Surface"), ("stroke", edge), ("cornerRadius", "Radius/M")])?;
        let value = themed(k, field, "Value", "Body/M", "Text", json!({"sizingH": "fill"}))?;
        themed(k, c, help, "Body/S", help_ink, json!({}))?;
        labels.push(label);
        values.push(value);
        variants.push(c);
    }
    let set = k.variants("Input", &variants, json!({}))?;
    k.text_prop(set, "Label", "Label", &labels)?;
    k.text_prop(set, "Value", "Value", &values)?;
    Ok(set)
}

/// A card component holding a button instance, with Title and Body text properties.
fn card(k: &mut Kit, page: NodeId, button: NodeId) -> Result<NodeId> {
    let c = surface(k, "component", page, "Card", V, [16.0, 24.0], "Surface raised", json!({"sizingH": "fixed", "width": 320, "y": 480}))?;
    let words = k.stack(c, "Words", V, 6.0, json!(0), json!({"sizingH": "fill"}))?;
    let wrap = json!({"sizingH": "fill", "textAutoResize": "height"});
    let title = themed(k, words, "Card title", "Heading/S", "Text", wrap.clone())?;
    let body = themed(k, words, "A short line that says what this card is about.", "Body/M", "Text muted", wrap)?;
    k.instance(button, c, json!({"Label": "Open"}), json!({}))?;
    k.text_prop(c, "Title", "Card title", &[title])?;
    k.text_prop(c, "Body", "A short line that says what this card is about.", &[body])?;
    Ok(c)
}

/// The components in use: a sign-in form, shown in one mode of the theme.
fn example(k: &mut Kit, page: NodeId, mode: &str, x: f64, button: NodeId, input: NodeId) -> Result<()> {
    let form =
        surface(k, "frame", page, &format!("Example · {mode}"), V, [20.0, 40.0], "Surface", json!({"sizingH": "fixed", "width": 400, "x": x}))?;
    k.mode(form, THEME, mode)?;
    let words = k.stack(form, "Words", V, 6.0, json!(0), json!({"sizingH": "fill"}))?;
    themed(k, words, "Welcome back", "Heading/M", "Text", json!({}))?;
    themed(k, words, "Sign in to plan your next walk.", "Body/M", "Text muted", json!({}))?;
    let fill = json!({"sizingH": "fill"});
    k.instance(input, form, json!({"Label": "Email", "Value": "mara@ridgeline.example"}), fill.clone())?;
    k.instance(input, form, json!({"State": "Focus", "Label": "Password", "Value": "••••••••••"}), fill.clone())?;
    let actions = k.stack(form, "Actions", V, 8.0, json!([8, 0, 0, 0]), fill.clone())?;
    k.instance(button, actions, json!({"Label": "Sign in"}), fill.clone())?;
    k.instance(button, actions, json!({"Type": "Ghost", "Label": "Create an account"}), fill).map(drop)
}
