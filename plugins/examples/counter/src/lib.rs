//! Counter: a widget on the canvas with − and + buttons around a count, a label you can edit on the
//! canvas, and a property menu (reset, steps of ten, a colour). Each click, edit or menu choice
//! calls the plugin back with the widget's state; it hands back the new state and the redrawn
//! widget. Its menu's "Set count…" opens a panel (API 5) whose Set button redraws the widget. An SVG
//! icon and a line (API 6) set off its label.

use figcraft_plugin_sdk::{Event, Input, Result, Value, Widget, close_ui, entry, json, render_widget, show_ui};

/// The count stays within ± this.
const LIMIT: i64 = 999_999;
/// The colours the menu offers for the count (the first is the default).
const COLORS: [&str; 4] = ["#1E1E1E", "#0D99FF", "#14AE5C", "#F24822"];
/// Shown in the label field while it's empty.
const PLACEHOLDER: &str = "Add a label";
/// Four tally marks and a stroke across them.
const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" fill="none" stroke="#757575" stroke-width="1.2" stroke-linecap="round"><path d="M2 2v8M4.5 2v8M7 2v8M9.5 2v8M1 9l10-6"/></svg>"##;

/// What the widget shows.
#[derive(Debug, PartialEq)]
struct Counter {
    count: i64,
    big: bool,
    color: String,
}

impl Counter {
    fn from_state(state: &Value) -> Counter {
        let color = state.get("color").and_then(Value::as_str).filter(|c| COLORS.contains(c)).unwrap_or(COLORS[0]);
        Counter {
            count: state.get("count").and_then(Value::as_i64).unwrap_or(0),
            big: state.get("big").and_then(Value::as_bool).unwrap_or(false),
            color: color.into(),
        }
    }

    /// After the click, edit or menu choice `w` brings (nothing: the widget was just inserted).
    fn next(mut self, w: &Widget) -> Counter {
        let step = if self.big { 10 } else { 1 };
        match (w.target.as_deref(), w.menu.as_deref()) {
            (Some("increment"), _) => self.count = self.count.saturating_add(step),
            (Some("decrement"), _) => self.count = self.count.saturating_sub(step),
            (_, Some("reset")) => self.count = 0,
            (_, Some("big")) => self.big = w.value.as_bool().unwrap_or(!self.big),
            (_, Some("color")) => {
                if let Some(c) = w.value.as_str().filter(|c| COLORS.contains(c)) {
                    self.color = c.into();
                }
            }
            _ => {}
        }
        self.count = self.count.clamp(-LIMIT, LIMIT);
        self
    }
}

/// A square button that calls back with `target`, darker while the pointer is over it.
fn button(target: &str, name: &str, label: &str) -> Value {
    json!({"type": "frame", "name": name, "target": target, "layout": "horizontal", "justify": "center", "width": 32, "height": 32, "radius": 8, "fill": "#F0F0F0",
           "hover": {"fill": "#E0E0E0"},
           "children": [{"type": "text", "text": label, "size": 16, "bold": true, "color": "#1E1E1E"}]})
}

/// The widget showing `c` under `label`.
fn view(c: &Counter, label: &str) -> Value {
    let (text, ink) = if label.trim().is_empty() { (PLACEHOLDER, "#B3B3B3") } else { (label, "#757575") };
    let (minus, plus) = if c.big { ("−10", "+10") } else { ("−", "+") };
    json!({"type": "frame", "name": "Counter", "layout": "vertical", "gap": 6, "padding": 12, "radius": 12, "align": "min",
           "fill": "#FFFFFF", "stroke": "#D9D9D9", "children": [
        {"type": "frame", "name": "Header", "layout": "horizontal", "gap": 6, "children": [
            {"type": "svg", "name": "Icon", "src": ICON},
            {"type": "text", "name": "Label", "field": "label", "text": text, "size": 12, "color": ink},
        ]},
        {"type": "line", "name": "Rule", "width": 120, "stroke": "#E6E6E6"},
        {"type": "frame", "name": "Row", "layout": "horizontal", "gap": 12, "children": [
            button("decrement", "Decrease", minus),
            {"type": "text", "name": "Count", "text": c.count.to_string(), "size": 24, "bold": true, "color": c.color},
            button("increment", "Increase", plus),
        ]},
    ]})
}

/// The property menu for `c`.
fn menu(c: &Counter) -> Value {
    json!([
        {"item": "reset", "label": "Reset", "type": "action"},
        {"item": "set", "label": "Set count…", "type": "action"},
        {"item": "big", "label": "Steps of 10", "type": "toggle", "checked": c.big},
        {"item": "color", "label": "Colour", "type": "color", "options": COLORS, "value": c.color},
    ])
}

/// The panel "Set count…" opens, showing `c`'s count.
fn set_panel(c: &Counter) -> Value {
    json!({"title": "Set count", "controls": [
        {"type": "number", "name": "count", "label": "Count", "min": -LIMIT, "max": LIMIT, "value": c.count},
        {"type": "button", "name": "set", "label": "Set"},
        {"type": "button", "name": "cancel", "label": "Cancel"},
    ]})
}

/// `c` after its panel's `e`: Set takes the count entered; Cancel changes nothing.
fn from_panel(mut c: Counter, e: &Event) -> Counter {
    if e.control == "set"
        && let Some(n) = e.values.get("count").and_then(Value::as_f64).filter(|n| n.is_finite())
    {
        // Panel numbers arrive as decimals; the bounds keep the cast exact.
        c.count = n.round().clamp(-LIMIT as f64, LIMIT as f64) as i64;
    }
    c
}

fn run(input: Input) -> Result<Value> {
    let w = input.widget.ok_or("Insert the counter from Plugins ▸ Widgets")?;
    let c = Counter::from_state(&w.state);
    let c = match &input.event {
        // The "Set count…" panel: Set redraws and closes it, Cancel only closes it, and typing in
        // its field waits for them.
        Some(e) if e.control == "set" => {
            close_ui();
            from_panel(c, e)
        }
        Some(e) => {
            if e.control == "cancel" {
                close_ui();
            }
            return Ok(json!({"count": c.count}));
        }
        None if w.menu.as_deref() == Some("set") => {
            show_ui(&set_panel(&c))?;
            return Ok(json!({"count": c.count}));
        }
        None => c.next(&w),
    };
    // An edited label is in the state already, under its field's name.
    let label = w.state.get("label").and_then(Value::as_str).unwrap_or("Counter").to_string();
    let state = json!({"count": c.count, "big": c.big, "color": c.color, "label": label});
    render_widget(&json!({"state": state, "root": view(&c, &label), "menu": menu(&c)}))?;
    Ok(json!({"count": c.count}))
}

entry!(run);

#[cfg(test)]
mod tests {
    use super::*;

    fn widget(target: Option<&str>, menu: Option<&str>, value: Value) -> Widget {
        Widget { target: target.map(Into::into), menu: menu.map(Into::into), value, ..Widget::default() }
    }

    #[test]
    fn counts_within_its_limit_and_follows_its_menu() {
        let c = |count: i64| Counter { count, big: false, color: COLORS[0].into() };
        let at = |n: i64, t: &str| c(n).next(&widget(Some(t), None, Value::Null)).count;
        assert_eq!(c(0).next(&Widget::default()).count, 0);
        assert_eq!((at(4, "increment"), at(4, "decrement")), (5, 3));
        assert_eq!((at(LIMIT, "increment"), at(i64::MIN, "decrement"), at(2, "other")), (LIMIT, -LIMIT, 2));
        assert_eq!(c(7).next(&widget(None, Some("reset"), Value::Null)).count, 0);
        let big = c(1).next(&widget(None, Some("big"), json!(true)));
        assert!(big.big);
        assert_eq!(big.next(&widget(Some("increment"), None, Value::Null)).count, 11, "steps of ten");
        assert_eq!(c(1).next(&widget(None, Some("color"), json!("#14AE5C"))).color, "#14AE5C");
        assert_eq!(c(1).next(&widget(None, Some("color"), json!("#123456"))).color, COLORS[0], "only the colours offered");
        assert_eq!(Counter::from_state(&json!({"count": 3, "color": "junk"})), c(3));
        let v = view(&c(3), "Score");
        assert_eq!((v["children"][0]["children"][1]["text"].clone(), v["children"][2]["children"][1]["text"].clone()), (json!("Score"), json!("3")));
        assert_eq!(view(&c(3), " ")["children"][0]["children"][1]["text"], PLACEHOLDER);
        assert_eq!(menu(&c(0))[2]["checked"], false);
        assert_eq!(v["children"][2]["children"][0]["hover"], json!({"fill": "#E0E0E0"}), "buttons darken under the pointer");
        assert!(run(Input::default()).is_err(), "it only runs as a widget");
        let ev = |control: &str, count: Value| Event { control: control.into(), values: json!({"count": count}), ..Event::default() };
        assert_eq!(from_panel(c(3), &ev("set", json!(42.0))).count, 42);
        assert_eq!(from_panel(c(3), &ev("set", json!(5_000_000))).count, LIMIT);
        assert_eq!(from_panel(c(3), &ev("cancel", json!(42))).count, 3);
        assert_eq!(set_panel(&c(3))["controls"][0]["value"], 3);
        assert_eq!(menu(&c(0))[1]["item"], "set");
    }
}
