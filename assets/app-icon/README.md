# FigCraft app icon

A **chameleon**: an engraving portrait in profile with a curled tail, in the Crafting Apps'
owl-template framing (full-bleed field, head and body filling the tile, body running off the edges).

## Palette

Exactly three colours:

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#0b0b0c` | line work and contours |
| Paper | `#efe9dc` | the figure |
| FigCraft green (app colour) | `#02da96` | the full-bleed field |

## Geometry

A 512-unit tile (`viewBox="0 0 512 512"`), rounded square with `rx=112`, no border. The macOS
renders pad it onto Apple's 824/1024 icon grid; Windows and Linux use the tile edge to edge, with
its rounded corners, like the other Crafting Apps.

## Provenance

The owner's original engraving-style drawing (1500 px, keyed to the palette), supplied for
FigCraft and vectorised. The source
image is not kept in this repo. License: see `LICENSE.txt`.

## Files

- `figcraft.svg`: canonical master (traced at 1500 px, the source's size).
- `figcraft-small.svg`: lighter trace (traced at 1024 px); also the hicolor scalable icon.
- `figcraft-<size>.png` (16–1024): renders on the macOS grid; `packaging/macos/package.sh` builds
  the `.icns` from them.
- `figcraft.ico`: Windows icon, 16–256 px, embedded in the `.exe` by `apps/figcraft/build.rs`.
- `hicolor/<size>/apps/ai.storyteller.figcraft.png` (16–512) and `hicolor/scalable/...svg`:
  Linux icon theme; the 256 px one is also the runtime window icon off macOS.

## Regenerate

Replace `figcraft.svg` (and `figcraft-small.svg`), then run `cargo xtask icon`. It needs `resvg`
(`cargo install resvg`).

## Nightly icons

The Nightly build ships the same chameleon on a deeper field (`#019668`), so the two icons are
told apart at 16 px without a badge, crescent or letter. Nothing is upscaled: `figcraft-1024.png`
is the only input, each frame is an area average of it, and only the field's contribution to each
pixel moves (ink, paper, shading and alpha are untouched).

- `figcraft-nightly.png`: the 512 px frame the Linux packages use.
- `figcraft-nightly-<size>.png` (16–1024): the frames; `packaging/macos/nightly.sh` copies the
  committed `figcraft-nightly.icns` into the bundle instead of building one on the runner.
- `figcraft-nightly.ico`: 32bpp DIB frames for 16–128 px and a PNG frame for 256; embedded in the
  Nightly `.exe` by `apps/figcraft/build.rs`.
- `figcraft-nightly.icns`: PNG slots for 16–1024, including the @2x ones.
- `nightly-provenance.json`: the input and output hashes, the palette, the method and the
  generator's own hash.

Regenerate with `python3 packaging/nightly-icons.py` (no image tool needed); `--check` reports
whether the committed files still match. `packaging/nightly-icon-tests.py` checks both containers,
the PNG CRCs and the provenance record.
