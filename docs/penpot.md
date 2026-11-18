# Penpot import and export

FigCraft reads and writes Penpot 2.x ZIP archives (`penpot/export-files`, version 1)
in Rust. Import and export run offline on desktop, CLI, MCP and WASM.

## Open and save

- **File → Open…** opens a `.penpot` as a separate unsaved document. Dropping it
  onto the canvas does the same. The original archive is not overwritten.
- **File → Import from file…** adds its pages to the current document.
- **Save / Save as…** writes native `.figcraft` files.
- **Export** with format `penpot` writes an archive for a Penpot team.

Engine commands use the same importer and exporter:

```json
{"command":"file.open","params":{"path":"example.penpot"}}
{"command":"file.import","params":{"path":"example.penpot"}}
{"command":"file.export","params":{"format":"penpot","path":"out.penpot"}}
```

Without a path, `file.export` returns base64 `data`. Unsupported conversions are
reported in `warnings`; the CLI prints them as notes.

```sh
figcraft-cli info example.penpot --json
figcraft-cli convert example.penpot editable.figcraft
figcraft-cli export editable.figcraft -o out.penpot --format penpot
```

## Supported content

| Area | Support |
|---|---|
| Structure | Pages, boards, groups, masks, nested frames, editable shapes and vectors |
| Appearance | Solid, gradient and image fills; strokes, shadows, blur, opacity, blending and transforms |
| Text | Paragraphs, styled runs, alignment, sizing, decorations and original font references |
| Layout | Constraints, guides, display grids and supported flex/grid layout with fixed, content-sized and fractional tracks |
| Components | Local main components, linked copies, overrides and variant sets; unresolved external copies remain independent with a warning |
| Tokens | Supported scalar tokens, aliases, theme modes, expressions and matching property bindings; literal compound typography/shadow tokens can become native styles |
| Prototypes | Flows, supported pointer/delay triggers, navigation, open/close overlays, back and links, with supported transitions |

Penpot source ids and selected source properties remain in layer metadata.
Fingerprints let export reuse unchanged text, layout, strokes, interactions and
tokens instead of regenerating them. Edited supported values are generated from
FigCraft's model. New layers receive deterministic UUIDs.

## Fonts

Embedded TTF/OTF/TTC/WOFF signatures are detected without decoding their payloads.
The current document model does not retain font binaries; import reports this
limitation. Install licensed fonts locally when needed. Native saves retain the
original family names and bounded provider metadata, rather than the binaries.

Provider references never trigger downloads. `file.fontReport` reports availability
and distinguishes Google provider references from custom library references whose
original binary cannot be verified. Missing fonts use local fallback while their
original family names remain editable.

Penpot export writes font references. Install the referenced fonts in the
destination Penpot team.

## Limits

- Penpot 1.x binary exports are unsupported; re-export from Penpot 2.x.
- Malformed archives, oversized members and excessive nesting/layer counts are
  rejected or bounded. Unsupported features produce import/export warnings.
- ZIP output is limited to 1 GiB of bytes actually read, with at most 16 MiB per
  JSON member and 128 MiB of shape JSON per page. All JSON members share a
  conservative four-million-value/key budget, checked before allocating parsed
  trees. Dense arrays cannot bypass it with small compressed or serialized sizes.
- Token expressions share nesting and work budgets across aliases and theme
  modes. Grid-cell membership is indexed once per parent.
- Raw SVG uses FigCraft's SVG importer where possible and retains source metadata.
  Complex masks and unsupported geometry may remain placeholders with warnings.
- Penpot export approximates unsupported paints/effects, custom dashes and
  transitions; unsupported actions and triggers are reported. Slices are omitted.
- Regenerated text has no Penpot `positionData`; Penpot lays it out again.
- An imported `tokens.json` is preserved when token changes cannot be regenerated;
  the exporter reports that limitation.
- Reverse flow, stretch alignment, wrapped-line space-around/space-evenly,
  negative padding, item margins/self-alignment and percentage tracks need richer
  model support. Their containers keep saved positions with warnings. Bounded
  source metadata survives native saves and unchanged Penpot re-export.
- Toggle-overlay has no equivalent in the current model. It is skipped with a
  warning, while unchanged source interactions remain available on re-export.
- Embedded font binaries are not retained by native saves or Penpot re-export.

## Verification

Regression tests cover supported editable content, unsupported-layout position
and source retention, offline font references, toggle-overlay warnings and the
archive/JSON/token budgets. These checks do not establish live Penpot server or
native macOS/Linux compatibility.

```sh
cargo xtask ci
cargo test -p figcraft-format penpot
cargo test -p figcraft-engine penpot
```

Optional local samples are not distributed or required:

```sh
FIGCRAFT_PENPOT_SAMPLE=/path/to/example.penpot cargo test -p figcraft-format penpot
FIGCRAFT_PENPOT_SAMPLES=/path/to/folder cargo test -p figcraft-engine penpot_export_tests
```
