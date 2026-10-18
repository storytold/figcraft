# Design tokens

FigCraft can work with **design tokens** in the [Design Tokens Community Group format
2025.10](https://www.designtokens.org/tr/2025.10/format/) instead of variables. Preferences ▸
**Work with** picks one: the Design panel, the bind pickers, the left rail and the editor follow
it. A file keeps its variables and its tokens either way, and both keep applying on the canvas.

## Model

- **Tokens** are kept as the DTCG JSON they are written in (`$value`, `$type`, `$description`,
  `$deprecated`, `$extensions`), so files round-trip with what other tools add.
- All 13 types: `color` (every colour space of the colour module, converted to sRGB for drawing),
  `dimension` (`px`, `rem` at 16 px), `number`, `fontFamily`, `fontWeight`, `duration`,
  `cubicBezier`, `strokeStyle`, `border`, `transition`, `shadow`, `gradient`, `typography`.
- Groups give their `$type` to their tokens; `$root` tokens and group `$extends` are read.
- Aliases: `{group.token}` and `$ref` JSON pointers, resolved after merging; cycles, missing
  targets and type mismatches are reported per token.

## Editing

- Every value has an alias button (on hover) that opens a searchable list of tokens of its type;
  picking one writes `{its.path}`. Typing `{group.token}` works too. An alias shows as a chip:
  click it to pick another, or detach it to keep the value it resolves to.
- Composite tokens fold to one row; the chevron shows each field on a row of its own, with its own field and alias button:
  typography (family, size, weight, letter spacing, line height), border, transition, shadow
  (each layer; add and remove layers) and gradient (each stop; add and remove stops).

## Themes and brands: sets and modifiers

The [resolver module](https://www.designtokens.org/tr/2025.10/resolver/) organises tokens:

- **Sets** always apply (`core`, `typography`…).
- **Modifiers** (`theme`, `brand`, `density`…) have **contexts** (`light`/`dark`,
  `acme`/`globex`…); one context per modifier applies at a time.
- The **resolution order** lists sets and modifiers; later ones override earlier ones.

The file chooses a context per modifier (the editor's header); frames, sections, components and
instances can choose another (Design panel ▸ Token contexts), inherited by their layers.

## Styles while working with tokens

With Design tokens on, styles a token type stands for step aside so the two don't compete:
the Fill, Stroke, Effects and Typography sections' style buttons open the token picker (colour
and gradient tokens; plus border tokens for strokes; shadow tokens; typography tokens), no paint,
effect or text styles are created from them, and the page's Local styles list only layout grid
styles. Styles already applied to layers keep working and can be detached.

## Binding

Bind buttons in the Design panel open the token picker (tokens whose type the field takes):

| Field | Types |
|---|---|
| fills / strokes colour, effect colour | color |
| fill gradient | gradient |
| width, height, radius, gap, padding, stroke weight, effect offsets/blur/spread, layout grids, font size, letter spacing | dimension, number |
| opacity | number (0–1) |
| line height | number (multiplier) or dimension |
| font family / weight | fontFamily / fontWeight |
| Typography header button | typography |
| Effects header button | shadow (replaces the layer's shadows) |
| Stroke header button | border (colour, weight, dashes) |

## Import and export

**Export ▸ ZIP** (the default) writes a bundle that follows the resolver module's multi-file
layout:

```
tokens.resolver.json                       resolver document: sets and modifiers point ($ref) at the files below
sets/<set>.tokens.json                     one token file per set
modifiers/<modifier>/<context>.tokens.json one token file per modifier context
```

Export also offers a single token file: one set or context as written, or the merged result for
the file's contexts with aliases resolved.

**Import** takes:

- a ZIP: its resolver document (the shallowest `*.resolver.json`) is followed; without one,
  `sets/<name>.json` become sets, `modifiers/<modifier>/<context>.json` contexts, and other JSON
  files sets named after the file;
- a resolver document on its own (its `$ref`s are read from the files next to it, desktop);
- a token file on its own: into a set named after the file, or (Import ▸ JSON file into this set
  or context) into the set or context shown;
- pasted JSON, and files dropped on the Design tokens window.

In the browser the file chooser reads the file's bytes (`token.import {data}`) and exports
download.

## Commands

`token.list`, `token.createSet`, `token.renameSet`, `token.removeSet`, `token.createModifier`,
`token.renameModifier`, `token.removeModifier`, `token.addContext`, `token.renameContext`,
`token.removeContext`, `token.setDefaultContext`, `token.moveOrder`, `token.set`, `token.remove`,
`token.rename`, `token.setContext`, `token.bind`, `token.import`, `token.export`,
`pref.designTokens`. `figcraft-cli commands` documents their parameters.
