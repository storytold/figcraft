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

The existing node-kind change carries appearances through snapshots, updates, server persistence,
undo and redo. Update participating clients and the server together: older versions understand
the editable text but may discard the optional appearance. Native `.figcraft` saves retain it;
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
