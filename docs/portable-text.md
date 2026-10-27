# Portable text appearance

Text remains editable characters and styled ranges. A machine with the named font faces also
saves positioned glyph outlines, decorations, line metrics and the text box in `TextData.appearance`.
A fingerprint covers the text and its layout settings, so geometry from an older edit is never
used for different characters or typography. No font file is embedded, installed or distributed.

Opening, saving, publishing and locally editing text capture appearances where the original
faces exist. `text.preserveAppearance {}` captures existing layers too, including a document that
was published before this feature. Run it on a machine with the original fonts. A collaborator
cannot recover the original appearance from a substitute font; missing faces are never captured
as if they were the originals. The command reports the number captured.

On a machine without the faces, valid saved outlines draw on the canvas and in SVG/PDF exports.
Its text boxes are not auto-sized in a substitute font. Moving layers preserves their geometry.
Editing preserved text or its typography requires installing the missing faces or explicitly
replacing them with installed ones. The Missing fonts window also checks joined server tabs.
Documents without saved geometry retain the previous fallback display; the warning says that
saved outlines apply where available. This does not repair a file whose text was already replaced.

Full node creations and snapshots carry appearances. Base-style metadata-only text-kind updates (hyperlinks and shared-style identifiers), with unchanged runs, omit
`appearance` and carry `unchangedAppearance: true`; the receiver applies only those metadata
fields, retaining its current characters, typography, run overrides and geometry even after a concurrent edit.
Changes to run metadata use the existing atomic full-kind update, avoiding a partial merge that
could silently discard links when concurrent edits split or merge run ranges.
A full kind update without the metadata marker replaces its appearance (null or omission clears it). Content/typography changes which regenerate geometry
send the complete appearance with the edited text in the same atomic property update. Moving
a layer, changing its node paint or updating base metadata does not resend unchanged glyph data.
Server persistence, undo and redo retain appearances. No message variant or property is added.
Update participating clients and the server together: older receivers may discard the optional
geometry on an omitted-field update. Native `.figcraft` saves retain the existing appearance;
the experimental `.fig` exporter does not currently write this extension.

Limits: 100,000 glyphs and 16 MiB of outline strings per layer. Colour bitmap/COLR glyphs currently
skip capture rather than silently losing their colour. Outlines preserve display, not font-based
editing. Font substitutions and missing styles still need the original fonts or an explicit
replacement. Different installed versions of the same named face are not detected.

Validation uses the current layout box, including sizes changed by layout or variable resolution.
Only dimensions affecting the selected auto-resize mode participate in the fingerprint. Both
commands and gesture previews reject changes to preserved text when its faces are absent; a
rejected gesture keeps the last valid document and cannot commit invalid geometry. Capturing
zero layers leaves the document identity and dirty state unchanged.

Starting a host (including local sharing) keeps newly captured geometry in the local tab and
sends that same document to peers, without changing whether the tab has unsaved edits.
Publishing captures the outgoing snapshot; its welcome installs that document locally before
editing becomes available. Hyperlinks and
shared-style identifiers do not invalidate outlines; actual rendering and layout inputs still
do. Automatic live-session font checks apply to joined guest tabs, not local hosts.

Validation: `crates/engine/tests/portable_text.rs` starts a separate process without system fonts.
It verifies identical rendered pixels and outlined SVG, PDF export, native save/reopen, protected
text editing and moving a layer. The synthetic face exists only in the author's process; neither
its font bytes nor private artwork are stored in test fixtures.
Review regressions also cover late font indexing before hosting/local sharing, clean and dirty
tabs, non-geometric style metadata, and the host's missing-font check queue.

Hosting prepares the captured snapshot before starting the link, and adopts it locally only
after startup succeeds. A rejected host/local-share start leaves both the local document and
its saved baseline untouched. Regressions cover successful and rejected starts with late font
indexing for clean and dirty tabs.

## Why capture for collaboration rather than export only?

A newly joined collaborator may not have the author’s fonts. The server’s document must already
contain enough display geometry to render the design before that collaborator edits or exports
anything. Capturing only at export time would not fix that live-document use case. This is
font-independent display, not permission to edit text using an absent font.

Appearances are immutable shared data in memory (`Arc`); cloning a text node, an undo entry or
fingerprinting editable inputs does not clone its outline strings. The serialized native shape
is unchanged. Exact named-face availability (including missing results) is bounded and cached:
a warm appearance check takes a read lock, not the font database’s write lock. Adding fonts and
both initial indexing and rescans invalidate these results, including late-installed styles.
The cache contains face names/results only, never font binaries.

Restored geometry takes hyperlinks from current editable character spans, not saved style links.
Removing or replacing a link therefore also updates missing-font PDF annotations. Saved line
glyph ranges must be ordered and non-overlapping, bounding cumulative traversal by glyph count;
PDF link collection also stops at its annotation/work budget independently.

Gesture previews normalize layout without capturing outlines on every disposable step. Commit
captures the final geometry before history/live synchronization; cancel retains the original.
Ordinary commands and typing still capture before synchronization. A synthetic 3,300-character
resize test measures eager capture versus deferred previews and verifies commit/undo/cancel.
On the local Windows optimized test build, 16 steps took 335.8 ms with eager capture versus
2.9 ms for deferred previews and 21.7 ms including commit. This synthetic measurement isolates
outline-capture overhead; it is not an end-to-end canvas or network benchmark.
