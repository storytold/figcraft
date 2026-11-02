# Design review

FigCraft's deterministic review of a design against the usual AI-generated UI tells. It reads a
selection, a page or the document and reports what a designer would fix first; `design.fix`
repairs the unambiguous findings as one undo step. There is no model, no network and no image
generation: this is a checklist with arithmetic, not a taste engine.

## Commands

| Command | What it does |
|---|---|
| `design.audit {scope?, rules?, minSeverity?}` | Reads the selection (default when something is selected), the page (default otherwise) or the document. Returns `score`, `counts`, `nodes` and `findings`; every finding has `rule`, `severity`, `node`, `path`, `message`, `fixable` and a `suggestion` where a safe one exists. Changes nothing. |
| `design.fix {scope?, rules?, dryRun?}` | Repairs the findings marked fixable in one undo step and reports `changed`, `skipped`, `byRule` and the counts before and after. `dryRun` reports without changing. Content, naming and gradients are left alone. |
| `design.rules` | Lists the rules with their severity, fixability and a one-line description. |

`scope` is `selection`, `page` or `document`; `rules` narrows the run to rule ids; `minSeverity`
is `low`, `medium` or `high`.

The same commands are available through the CLI (`figcraft-cli run`), the control channel and MCP
(`run_command`). The MCP server's initialize instructions name them, and `prompts/list` has the
`design_review` prompt: it runs the audit, fixes what is unambiguous, audits again and reports.
`VERIFY` in the design prompts does the same before an agent finishes a screen.

## The window

The checkmark button in the bottom toolbar — **Review design**, next to Actions — opens a movable,
resizable window, as do **Review design** in the Actions palette (⌘K, no search needed) and
`ui.menu.invoke {"id": "design.audit"}` on the control channel. Pick Selection, Page or Document
and press **Check design**: the window shows the score with a progress bar, the counts and every
finding, filterable by severity. **Select** selects the layer; **Fix** applies that rule; **Fix
all** applies every fixable rule. Repairs run `design.fix` and the list checks itself again. The
toolbar button closes the window again when it is open.

The window follows Preferences ▸ Language: its labels, severity names and rule titles are in the
interface catalogs (all 24 catalogs carry them; Chinese included). A finding's `message` and `path`
come from the engine and stay English, like command and MCP output; the rule id is on the label's
tooltip for use with `rules`.

## Prior art

The interaction model follows open-source design linters rather than inventing one: *Design Lint*
([MrSuazo/dsm-design-lint](https://github.com/MrSuazo/dsm-design-lint)) for click-to-select,
severity filtering and fix-all, and the *figma-use* linter
([dannote/figma-use](https://github.com/dannote/figma-use)) for rule categories, presets and JSON
output. No code, text or asset is copied; the rules are FigCraft's own.

## What it checks

| Rule | Severity | Fixable |
|---|---|---|
| `spacing.off-grid` — gap or padding off the 4px grid | medium | yes |
| `radius.off-scale` — a corner not on the 4px scale or a pill | low | yes |
| `type.sizes` — more than five sizes in one scope | medium | no |
| `type.families` — more than two families in one scope | medium | no |
| `type.weight` — a heading heavier than 600 | low | yes |
| `type.italic` — italic interface text | low | yes |
| `type.too-small` — body text below 11px | medium | no |
| `type.negative-tracking` — small text tracked tighter | low | yes |
| `color.gradient` — a shape painted with a gradient | medium | no |
| `color.gradient-text` — text painted with a gradient | high | no |
| `color.glow` — a shadow with no offset and a large blur | medium | yes |
| `color.colored-shadow` — a shadow with a visible hue | low | yes |
| `color.border-opacity` — a border stronger than a hairline | low | yes |
| `color.contrast` — text below 4.5:1 (3:1 large) | high | yes |
| `icon.emoji` — emoji in text or a layer name | low | no |
| `copy.exclamation` — an exclamation mark in interface copy | low | no |
| `copy.trailing-period` — a short label ending in a period | low | no |
| `copy.hype` — "AI-powered", "supercharge" and the like | low | no |
| `naming.auto` — a layer still named "Frame 12" | low | no |
| `detail.shadow-count` — shadows on too many layers | low | no |
| `layout.text-overflow` — a fixed-width text box shorter than its text | high | no |

The pair to remember: **hi**gh is a blocker (contrast, gradient text, clipped text), **medium** is
a discipline problem (spacing, type, gradients), **low** is polish (weights, borders, names, copy).
A clean screen has no high and at most two medium findings.

## What it deliberately does not do

- **No direction, copywriting or composition.** Choosing what a product should feel like, rewriting
  a headline or inventing a hero visual needs a model (or a person); the audit can only report what
  is measurable. For that layer use the MCP agent loop with `design_review`, or a person.
- **Gradients, naming and copy are report-only.** They need a decision, not a formula.
- **Contrast is only judged on a known opaque solid backdrop.** A gradient, image, translucent layer
  or partial cover makes it skip the check rather than guess. Backgrounds are found through the
  paint order (siblings below the text, groups and containers included, `reverse_z` respected).
- **Text overflow is measured with the text layout engine.** A font that lays out differently than
  it renders can still clip; the screenshot is the last word.
- **The rules are heuristics.** A finding is a question ("does this border need to be this strong?"),
  not a verdict. The score is a summary, not a grade.

## Credits

The rules are FigCraft's own reading of the public unslop checklist by Damir Sokolovsky
([heydamir/unslop](https://github.com/heydamir/unslop), CC BY 4.0). No text, image, script or other
asset from it is copied into FigCraft.
