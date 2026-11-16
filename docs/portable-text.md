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
fingerprinting editable inputs does not clone its outline strings. The compact wire/native
encoding is described below; readers also accept the earlier positioned-path records. Exact named-face availability (including missing results) is bounded and cached:
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


## Reproducible text-heavy storage benchmark

Run `cargo xtask bench text 1000,10000` with the normal `CRAFT_FONTS_DIR` build input.
This is a separate storage workload, not the mixed shapes/text canvas benchmark. It uses
unique 117-character paragraphs, two embedded Inter faces (Regular/Bold) at two sizes,
fixed text boxes, and no images, version history or external font dependency. System font
scanning is disabled by the xtask command. No private document or font binary is a fixture.

Each size runs one warm-up followed by five measured samples; the table reports medians.
`prepare` times session initialization of a document without appearances (including initial
capture on this branch), separately from saving. `first snapshot` then times `file_bytes`
(thumbnail and native ZIP encoding). `repeat snapshot` is the same save path after reopening
those bytes, so valid appearances are already present. `reopen` times native ZIP/JSON loading
and session initialization, not GUI startup, canvas rendering, networking or filesystem reads. The saved
byte count includes the thumbnail. These are memory-I/O measurements without disk flushes;
"first" does not mean a cold process or cold filesystem cache. Text layer counts are checked
after every load; the portable-layer column counts stored appearances, and must match the
text count on this branch (zero on unmodified main).

For a before/after comparison apply only the CLI/xtask benchmark harness to the chosen main
revision, use the same release profile, fonts and machine, and run the two variants sequentially.
Keep the exact revisions and results together: capture and serialization costs depend on the
text/font workload, and a resize microbenchmark does not answer the load/save question.


### Earlier positioned-path baseline, 2026-11-12

Base: `0d6ce1124c03c8efc3807db5689fee7f8ec94756` (main), with only this benchmark harness applied.
Feature: portable-text integrated with that same main, measured with the harness in this change.
Windows x64, Intel Core Ultra 9 285H, 32 GiB RAM; Rust 1.99.1; repository-default release
profile (thin LTO, four codegen units), four Cargo build jobs. The same craft-fonts input
(`37827adc41fd165f9a9234ac40f94dbad353aa69`) was used for both builds. They ran sequentially.
All times below are medians in milliseconds; sizes are exact ZIP byte counts.

| Text layers | Variant | Prepare ms | First save ms | Repeat save ms | Reopen ms | Saved bytes | Stored appearances |
|---:|---|---:|---:|---:|---:|---:|---:|
| 1,000 | main | 0.267 | 108.054 | 102.384 | 4.759 | 127,908 | 0 |
| 1,000 | portable text | 720.823 | 1802.479 | 1748.249 | 520.258 | 29,457,300 | 1,000 |
| 10,000 | main | 3.337 | 1677.961 | 1683.007 | 58.113 | 265,376 | 0 |
| 10,000 | portable text | 8380.447 | 19743.431 | 20262.586 | 5891.669 | 294,632,582 | 10,000 |

The 10,000-layer document contains 1,170,000 characters. All text layers survived every load;
the feature stored an appearance for every layer (main stored none). That earlier representation
was **not within the requested 10% overhead target**: at 10,000 layers, repeat save is **12.04×**, reopen **101.38×**, and ZIP size **1110.25×** main.
The roughly 294.6 MB archive is a text-only synthetic case; it is not a measurement of a user's
image-heavy file. These results expose the cost of per-positioned-glyph SVG strings in native
serialization. Shared in-memory geometry and deferred gesture capture do not remove that file
size or load/save cost. Further storage-representation work or a different capture policy is
needed before claiming the performance criterion is satisfied; see the compact encoding follow-up below.


## Compact encoding and migration

`appearance.encoding: 1` (read compatibility) stores a self-contained `outlines` table and glyph tuples:
`[outlineIndex, x, y, size, advance, charStart, charEnd, style, rtl, placement]`.
Shapes are saved in their original font units, once for every distinct path in that appearance.
Ordinary placement is one scale number (the tuple already has x/y); text on a curve keeps all
six affine coefficients. Null placement means an earlier, already-placed path. No coordinate
rounding or lossy simplification is used. Capturing shares outline strings across layers for
one capture operation, bounded to 20,000 entries and 16 MiB; restoration parses a repeated shape
only once per layout. Version-one glyph tuples are read directly, without buffering a generic JSON tree.
Current writers use encoding 3: the same outline table plus a lossless packed `glyphs` string
described below.

Each text node remains independently transferable in collaboration, clipboard and history;
there is no live-document table that another client's edit could invalidate. The reader
accepts the earlier object-shaped glyph records without `encoding` as version zero. With the
source faces installed, capture replaces valid version-zero positioned paths with normalized
geometry; without them, the earlier geometry remains exact and can still be saved and displayed.
The new encoding alone cannot remove position-dependent duplicates from an old file whose
fonts are absent. Neither reader nor writer embeds a font binary.

Unknown encodings, invalid table references, more than 100,000 glyphs/outlines, more than 16 MiB
of dictionary strings (including unused entries), or more than 16 MiB of expanded path strings
are refused. Display additionally checks finite transforms, paths, ranges and metrics.
Upgrade experimental clients and servers together: builds of this PR before compact encoding
may reject the new records. New readers open the earlier records. Experimental `.fig`
export still does not write portable appearances.


### Native file table

Native `document.json` also carries `textOutlines: {version: 1, paths: [...]}`. During
serialization only, its local appearance dictionaries use `@<index>` strings into that file
table. The writer clones only affected text nodes in a temporary document; their glyph lists remain
shared and an explicitly scoped outline palette substitutes references during encoding. It does not change
the live document, fingerprints, undo, or collaboration state. Loading resolves references
before returning a document or validating its editable content. A saved version or branch
snapshot has its own table. Plain in-memory/collaboration serialization stays self-contained.

The file table is bounded to 100,000 paths and 64 MiB. Shapes that do not fit are written inline,
so the bound never silently discards content. Missing references and unknown versions are
refused. Resolved paths are charged against both the 16 MiB per-layer limit and the native
document work limit (4 GiB on native 64-bit, 1 GiB on 32-bit). References are resolved once,
not recursively. Existing archive member, compression-ratio and total unpacking caps remain.


### Packed numeric records (encodings 2 and 3)

`glyphs` is standard padded base64, not an image. Encoding 3 first expands its bounded zero-run
stream (described below); encoding 2 uses the base64-decoded bytes directly. In that inner
column stream, the first u32 is
record count. Four u32 columns follow (outline index, char start, char end, style), then four
IEEE-754 f64 columns (x, y, font size, advance). Each column writes byte plane 0 for all rows,
then plane 1, and so on. This preserves the exact bits while exposing repeated low/high bytes
to ordinary ZIP compression, instead of repeatedly formatting/parsing decimal numbers.
Encoding 3 keeps the same column layout, but the four numeric columns contain XOR residuals
against a prediction (zero before the first row). For x the next prediction is the decoded x
plus decoded advance; for y, size and advance it is the previous decoded value. Predictions
which overflow are replaced by positive zero. XOR uses the complete 64-bit IEEE representation,
including negative zero: no coordinate is rounded or reconstructed through subtraction.
Character starts are XORed with the previous character end, ends with current start plus one
(wrapping u32), and style indices with the previous style; the first previous values are zero.
Placement payloads likewise XOR each component with that component's previous bits (zero before
the first payload; absent placements do not update them). The same finite sanitization applies
before forming all predictions. Arbitrary jumps, overlapping ranges, ligatures and affines remain
exact residuals. Encoding 2 remains readable with its original unpredicted columns and payloads.

Next are one RTL byte per row (0/1), one placement tag per row (0/1/2), and placement payloads
in row order: none, one scale, or six affine coefficients. Encoding-2 payloads are ordinary
little-endian f64s; encoding 3 uses the predicted bits described above. No lossy quantization or font re-shaping is needed to decode them.

Encoding 3's outer run stream starts with the inner byte length as a little-endian u32. A
high-bit tag appends `(tag & 127) + 1` zero bytes; other tags copy `tag + 1` following literal
bytes. The writer only makes zero runs of at least three bytes (short runs remain in literals),
so worst-case encoding is bounded by the inner size plus one tag per 128 bytes and the header.
Expansion is refused beyond the normal 100,000-glyph byte budget, on a truncated literal,
on a run crossing the declared size, or if the final size differs. Decode does not allocate
based on an unbounded length claim or defer work until after the reopen measurement.

The decoder bounds encoded/raw lengths, record count, every column/payload read and indices;
it refuses invalid flags, non-finite values, truncation and trailing bytes. Writing applies
the native finite-number policy (non-finite values become zero). Old encoding-zero objects
and encoding-one tuples are still accepted. Font-unit paths and editable characters remain
ordinary document data; the glyph number encoding contains no font binary.


### Lossless packed follow-up, 2026-11-12

Same corpus, Inter input, release profile and one warm-up/five measured medians as above.
The unoptimized portable baseline is PR head `194909935faf18753d5fe4dd7e810c377a2cb66d`.
The compact variant includes normalized shared shapes, the native file dictionary, and encoding-2
byte-plane glyph numbers. A fresh sequential main run uses the already-built main binary from
`0d6ce1124c03c8efc3807db5689fee7f8ec94756`; release compilation is excluded from all timings.

| Text layers | Storage variant | Prepare ms | First save ms | Repeat save ms | Reopen ms | ZIP bytes | Appearances |
|---:|---|---:|---:|---:|---:|---:|---:|
| 1,000 | main (fresh run) | 0.292 | 122.237 | 119.880 | 5.271 | 127,908 | 0 |
| 1,000 | portable, positioned SVG | 720.823 | 1802.479 | 1748.249 | 520.258 | 29,457,300 | 1,000 |
| 1,000 | portable, pooled/packed | 14.447 | 205.341 | 204.655 | 56.744 | 939,721 | 1,000 |
| 10,000 | main (fresh run) | 3.343 | 2085.005 | 2053.145 | 63.888 | 265,376 | 0 |
| 10,000 | portable, positioned SVG | 8380.447 | 19743.431 | 20262.586 | 5891.669 | 294,632,582 | 10,000 |
| 10,000 | portable, pooled/packed | 975.043 | 3021.400 | 2860.561 | 528.367 | 8,275,682 | 10,000 |

At 10,000 layers / 1,170,000 characters, the optimized archive is **35.60 times
smaller (97.19% fewer bytes)** than the earlier portable implementation.
Measured repeat save is **7.08 times faster** and reopen **11.15 times faster**.
All 10,000 appearances and text layers survived every round trip. No image rasterization,
coordinate quantization or font substitution accounts for these reductions.

The original review's roughly 10% overhead target versus main is **still not met**: repeat
save is 1.39 times main, reopen 8.27 times, and ZIP size 31.18 times.
These are local synthetic text-only measurements, not a guarantee about GUI/network latency
or an image-heavy customer document. The earlier and fresh main timings also show normal
run-to-run machine variation; no confidence interval or controlled cross-machine speedup is claimed.


### Compression locality and decoding copies

The node map serializes ordinary nodes in id order, followed by portable text nodes grouped by
base font family/style, exact size/width bits and id. Layer stacking/page order remains in the
unchanged `children`/`pages` arrays. This is deterministic and only portable nodes are sorted;
a file without appearances retains its previous map order without a sorting allocation.
Grouping lets adjacent similar packed layouts share Deflate's 32 KiB window. The native writer
also memoizes shared outline allocation identities within the bounded packing operation, with
content equality retained for separately allocated identical paths. Reading packed records
constructs final model glyphs directly in a preallocated vector without intermediate tuple/enum
vectors, and loads byte planes by bounded indexing instead of allocating transposed columns. These optimizations do not omit appearances, relax validation, or defer decoding out of
the measured reopen path.

Because compressed records can expand far beyond their input strings, document node decoding
also charges estimated retained glyph structs and decoded path dictionaries per layer against a cumulative
1 GiB budget on 64-bit targets (256 MiB on 32-bit). It refuses the next layer before retaining it
past the limit, rather than waiting for the complete document to be materialized. Per-layer
record/path limits still apply independently; at most one bounded layer is decoded before its
aggregate charge is checked. Repeated references within one layer are not counted as duplicate
path allocations; unused entries are conservatively charged. This count is obtained by the
decoder, avoiding another glyph scan or pointer set while reading each node. Ordinary nodes do not consume this geometry budget. Native opens also share an expanded
glyph-struct budget across the main document, eagerly parsed branches and published library;
version-history JSON stays lazy. Additional snapshots past the budget are skipped with a reason,
using the existing damaged-snapshot reporting path.


Glyph lists use copy-on-write shared storage. A native encoding snapshot attaches one shared
outline-reference palette; source nodes and positioned glyph lists are unchanged. Mutation
detaches storage and removes that encoding palette. Pointer lookups only refer to allocations
kept alive by owned path keys, with content-equality fallback for independently allocated paths.
There is no global/thread-local serializer state. Native files, node updates and clipboard data
retain the same wire content as before this sharing optimization.


### Predicted records and shared snapshots, 2026-11-12

Same synthetic corpus, Inter input, release profile and one warm-up/five measured medians.
Main is the unchanged binary from `0d6ce1124c03c8efc3807db5689fee7f8ec94756`, run immediately
after the current feature binary, without a concurrent build. The previously published compact
implementation is `34d2f5183932f23b94eaa8cd5d89046d6c108c0c`. Reopen still includes full decoding
and session initialization; no work was deferred beyond that measurement.

| Text layers | Variant | Prepare ms | First save ms | Repeat save ms | Reopen ms | ZIP bytes | Appearances |
|---:|---|---:|---:|---:|---:|---:|---:|
| 1,000 | main (fresh run) | 0.289 | 116.398 | 116.118 | 4.972 | 127,908 | 0 |
| 1,000 | previous pooled/packed | 14.447 | 205.341 | 204.655 | 56.744 | 939,721 | 1,000 |
| 1,000 | predicted/shared | 13.450 | 152.359 | 150.058 | 43.477 | 251,186 | 1,000 |
| 10,000 | main (fresh run) | 3.135 | 1586.164 | 1596.029 | 56.433 | 265,376 | 0 |
| 10,000 | previous pooled/packed | 975.043 | 3021.400 | 2860.561 | 528.367 | 8,275,682 | 10,000 |
| 10,000 | predicted/shared | 784.513 | 1885.323 | 1874.355 | 429.437 | 1,343,321 | 10,000 |

Compared with the previously published compact storage, the 10,000-layer file is
**6.16 times smaller**, repeat save is **1.53 times faster** and reopen is
**1.23 times faster** in these samples. Every appearance is retained.

Versus fresh main, first save is **+18.9%**, repeat save **+17.4%**,
reopen **7.61x**, and file size **5.06x** main. The approximately 10% review
target is not met for all metrics. Saved geometry and per-layer fingerprints remain additional
data that main does not store. These synthetic measurements do not guarantee GUI/network or
image-heavy file performance. Normal machine variation applies to timing comparisons.


### Opening follow-up, 2026-11-13

A diagnostic build of the previous revision instrumented native header parsing, document
deserialization, appearance wire/number/model conversion, outline-pool expansion, validation
and session initialization. At 10,000 layers it spent roughly 330–350 ms constructing the
document, 14–15 ms on the header, 28–30 ms expanding the pool, 5–6 ms validating and 20–22 ms
initializing the session; ZIP/other load work made up the remainder. The wire read (~50 ms),
packed numeric decode (~60 ms) and conversion into model glyphs (~120 ms) are **nested inside**
document construction, not extra stages to sum with it. These temporary timers were removed
from the production code and the final benchmark.

The packed decoder now validates and builds final model glyphs in one vector preallocated to
the bounded record count, instead of first producing tuples and an enum iterator and growing
another result vector. Numeric byte planes use direct bounded indexing instead of constructing
chunk iterators per value. Local outline strings are allocated directly as shared strings,
avoiding generic String/Box temporaries; the dictionary uses a bounded sequence-size hint.
A direct-construction diagnostic prototype reduced the combined numeric/model stage from
about 185–190 ms to 108–115 ms. The change does not defer validation/materialization or alter
encoding 3, native table 1, coordinates, ordering or retained geometry. Missing packed paths
and expanded-path overflow remain errors, with regression coverage.

The fixture contains 1,170,000 characters/glyph placements. Its native JSON is 25,607,910 bytes
after ZIP expansion. A synthetic control copy with only `appearance` and `textOutlines` removed
contains approximately 6,379,922 JSON bytes after rewriting; this control is not the vanilla-main
archive. The remaining load difference includes processing this larger metadata/number payload
and constructing all saved geometry; main stores editable text without those glyph records.

Final production builds, without diagnostic timers: same corpus/toolchain/Inter input and one
warm-up plus five measured medians, run sequentially without a concurrent build. The previous
production binary corresponds to `759565f83c8e4546badf207625753c891363a66b`; main remains
`0d6ce1124c03c8efc3807db5689fee7f8ec94756`. Reopen includes all decoding and session setup.

| Text layers | Variant | First save ms | Repeat save ms | Reopen ms | ZIP bytes | Appearances |
|---:|---|---:|---:|---:|---:|---:|
| 1,000 | main (fresh) | 114.679 | 114.118 | 4.984 | 127,908 | 0 |
| 1,000 | previous portable | 161.768 | 159.841 | 43.197 | 251,186 | 1,000 |
| 1,000 | direct model decode | 156.389 | 156.354 | 37.108 | 251,186 | 1,000 |
| 10,000 | main (fresh) | 1878.315 | 1892.246 | 61.297 | 265,376 | 0 |
| 10,000 | previous portable | 2526.593 | 2271.747 | 484.186 | 1,343,321 | 10,000 |
| 10,000 | direct model decode | 2304.325 | 2317.884 | 375.279 | 1,343,321 | 10,000 |

The 10,000-layer reopen time falls from **484.186 to 375.279 ms (22.5% less time)**.
All appearances survive every round trip; saved byte counts are unchanged. Read-only fixes do
not target saving, whose small timing differences remain subject to normal machine variation.
Versus the fresh main sample, reopen is **6.12x** (previously 7.90x in this pair),
repeat save **+22.5%**, and archive size **5.06x**. The roughly 10%
default-path acceptance target remains unmet; no opt-in policy or font-preservation semantics
were changed. These measurements exclude filesystem fsync, network and GUI startup.


### Compact JSON metadata follow-up, 2026-11-13

Appearance line records now serialize as twelve-element arrays in the existing `TextLine`
field order: `glyph_start, glyph_end, char_start, char_end, top, height, baseline, width, x,
para_end, break_after, rtl`. The existing derived reader accepts both these arrays and named
objects. Appearance styles omit only stable defaults: auto line height, positive-zero percent
letter spacing, no decoration, original case, absent optional fields and empty variation axes.
Font family, style and size are always explicit because font defaults can differ between builds.
Nondefault styles, explicitly empty fills, links and negative-zero values stay explicit. These
changes apply only to saved appearance metadata, not editable text styles or fingerprints.
Encoding 3 and native outline table 1 are unchanged; no extra reader or migration is required.
Exhaustive field destructuring catches newly added model fields instead of silently dropping them.

All glyph byte strings and local/native outline palettes are untouched. A native rewrite of
all 10,000 layers was compared to the previous fixture after restoring the declared metadata
defaults and field names: the complete document, including every floating-point bit, matched.
The previous production binary (`e6d6fc7545b94648b00ef44c49a5cee93e17bfeb`) opened the new native
file successfully without source system fonts. Reading and resaving it with that previous
production binary reconstructed the original 25,607,910 JSON bytes exactly (SHA-256
`de943c9a310480d04c9900d787b306f75cca07268e03860533c336a0922148a4`).
A regression uses the old derived reader types,
nondefault styles and precise/signed-zero numbers through an actual JSON string round trip.

Fresh release runs use the same corpus, toolchain, fonts, one warm-up and medians of five
samples as above, sequentially without a concurrent build. The before binary is that same
`e6d6fc75`; the unchanged main binary is `0d6ce112`. Reopen still decodes and initializes all
geometry before its timer stops.

| Text layers | Variant | First save ms | Repeat save ms | Reopen ms | ZIP bytes | Appearances |
|---:|---|---:|---:|---:|---:|---:|
| 1,000 | main (fresh) | 112.700 | 110.301 | 4.908 | 127,908 | 0 |
| 1,000 | previous portable | 148.696 | 152.753 | 34.252 | 251,186 | 1,000 |
| 1,000 | compact metadata | 158.349 | 155.107 | 33.696 | 245,029 | 1,000 |
| 10,000 | main (fresh) | 1748.223 | 1784.232 | 61.772 | 265,376 | 0 |
| 10,000 | previous portable | 2247.750 | 2214.062 | 378.614 | 1,343,321 | 10,000 |
| 10,000 | compact metadata | 2179.060 | 2156.879 | 337.938 | 1,286,854 | 10,000 |

At 10,000 layers, expanded native `document.json` falls from **25,607,910 to 21,222,910 bytes
(17.1% smaller)**; ZIP size falls **4.2%**. Reopen takes **10.7% less time** and repeat save
**2.6% less time** in this sample. Small timing differences remain subject to machine variation;
there is no cross-machine guarantee. All 10,000 appearances / 1,170,000 glyph placements remain.
Versus fresh main, reopen is **5.47x**, repeat save **+20.9%**, and archive size **4.85x**.
The approximately 10% default-path target remains unmet. No validation/materialization was
deferred, and no geometry precision, source-font policy or capture behavior was changed.


### Save/open memory consistency, 2026-11-16

A native save now refuses portable geometry that the same reader would reject for its decoded
memory budget. The document preflight runs on the actual encoding snapshot: glyph record bytes
plus the distinct per-layer outline dictionary, using native references where pooling applies.
It does not count every occurrence of a shared live SVG as a separate dictionary entry. The
calculation uses the same record size and exact encoded string lengths as the compact reader.
No appearance is removed to make the file fit, and the reader's existing caps remain unchanged.

Before returning an archive, saving also charges the main document, published library and branch
base/head snapshots against the same aggregate numeric glyph budget used during eager loading.
In-memory version snapshots receive the per-document check when encoded; stored history JSON
remains lazy. A refusal returns an error before native-file replacement, preserving unsaved edits
and the prior disk file. This does not add recovery by dropping geometry or relax any load limit.

Synthetic regressions use a small injected save budget rather than allocating a gigabyte: a
below-budget control saves and reopens with unchanged geometry; a four-node document is refused;
the exact pooled-dictionary boundary succeeds and one byte less fails; branch/library snapshots
share the eager budget while history remains lazy. An accounting regression compares the writer's
estimate with the actual compact decoder, including equal paths from separate allocations and
native outline references.

The branch is integrated with main `559c952d` (2026-11-15): native envelopes, image resources and
archive byte budgets still use the newly shared `craft-figcraft-format` implementation. Portable
metadata and pooling wrap its app-supplied payload, and a local wrapper adds the glyph budget to
its archive reader. The opening-page and relative-library-link save behavior from main is kept.
The earlier performance tables are historical results on their stated bases, not fresh timings
for this integration. Working application packages, deployed services and artwork are unchanged.


### Streaming native outline table, 2026-11-16

After the current-main integration, opening made three passes over native document JSON: the
shared envelope/version check, ordinary document deserialization, then a separate outline-table
read. The app payload now intercepts only the root `textOutlines` field during the ordinary
streaming document read. The shared header/version pass remains first. Pool expansion and all
validation/session initialization still finish before reopen returns; no work is deferred.

A MapAccess adapter forwards every other field/value directly to the existing Document reader.
There is no serde-flatten/Value/Content tree buffering the nodes before their resource checks.
Only root keys pass through the adapter; nested unrelated fields are ignored as before. The pool
uses its existing bounded deserializer, including duplicate-key rejection even after null. Plain
self-contained documents remain readable. Glyph codecs, exact numbers, native tables, writes,
per-document geometry caps and shared eager-snapshot caps are unchanged.

Regressions cover a pool before/after the document fields, escaped root keys, unrelated nested
keys, precise values and signed-zero affines, duplicate tables including null, excessive table
entry counts in either order, and the shared newer-version check before an invalid app payload.
The existing native-save boundary/snapshot and missing-font checks remain part of full CI.


Fresh release measurements on the same Windows x64 host, Rust 1.99.1 and embedded Inter input
use the existing `cargo xtask bench text 1000,10000` corpus (117 chars/layer, no images/history).
Before is production `9fe7c9a7`, already based on main `559c952d` with save-budget fixes.
The harness is unchanged: one warm-up and medians of five samples, in-memory native save and
complete decode/session initialization for reopen. After building both binaries, the repeated
order was before/after/before without concurrent Rust builds. The first after sample follows its
build; the initial before sample is listed to show host variation rather than hide it.

| Text layers | Run | First save ms | Repeat save ms | Reopen ms | ZIP bytes | Appearances |
|---:|---|---:|---:|---:|---:|---:|
| 1,000 | initial before | 265.193 | 261.616 | 34.123 | 245,063 | 1,000 |
| 1,000 | initial after | 245.230 | 244.593 | 30.429 | 245,063 | 1,000 |
| 1,000 | paired before | 242.784 | 252.401 | 31.969 | 245,063 | 1,000 |
| 1,000 | paired after | 234.495 | 232.817 | 29.911 | 245,063 | 1,000 |
| 1,000 | before control | 259.559 | 249.425 | 34.115 | 245,063 | 1,000 |
| 10,000 | initial before | 4256.658 | 4336.941 | 356.266 | 1,286,888 | 10,000 |
| 10,000 | initial after | 4010.881 | 3968.139 | 310.363 | 1,286,888 | 10,000 |
| 10,000 | paired before | 3932.824 | 4058.045 | 329.432 | 1,286,888 | 10,000 |
| 10,000 | paired after | 2825.718 | 3258.381 | 327.243 | 1,286,888 | 10,000 |
| 10,000 | before control | 4410.214 | 4329.622 | 354.115 | 1,286,888 | 10,000 |

The adjacent 10,000-layer pair is **329.432 → 327.243 ms (0.66% less time)**.
The first pair shows a larger difference, but before/control timings also move substantially.
This is not evidence of a reliable double-digit reopen improvement. Writes are unchanged;
the large save-time movements are not claimed as a benefit of this reader-only change.

A separate read-only CLI probe uses the existing `run` command and the same native fixture:
one warm-up process per variant, then six processes per variant in alternating AB/BA order.
Each process performs 16 `file.open`/forced `file.close` pairs. Timings are process wall time
per pair, including startup, filesystem reads, session/batch/cache work and close cleanup;
they are not the in-memory reopen metric above. No document is saved by this probe. Its
median of six process averages is:

| Variant | Median ms per open/close | Min–max process average ms |
|---|---:|---:|
| before | 390.802 | 365.694–396.885 |
| streaming | 364.096 | 350.570–452.869 |

That probe's median is **6.83% lower**, but the ranges overlap.
The extra JSON pass is removed structurally; a stable percentage speedup is **not established**
on this noisy host. No fresh vanilla-main baseline or new default-path acceptance claim is made.

All 10,000 appearances / 1,170,000 glyph placements remain in every storage benchmark.
Reading and resaving the same fixture with each production binary produces **identical native
archives (1,286,888 bytes)** and identical document JSON (21,222,910 bytes),
SHA-256 `5b0dcb8274a55acba9d03252c6114115a637382e6eb82935b2bf68e32da47ec9`.
This preserves all encoded glyph data, outlines, exact numeric values and other document fields.
The native/wire versions, writer and working app/server packages are unchanged.


### Fresh main and glyph-stage follow-up, 2026-11-16

The current upstream main is `559c952d107f781c4f770041c7d5cc368ebc4fa0`; the production
before revision is `175f58b8`. Both were built with the same release profile, Rust 1.99.1,
Windows x64 host and craft-fonts input. Only the existing text-storage CLI harness/argument
and dispatch files were backported to main for measurement: no portable behavior or model
changes. The temporary checkout/harness edits were restored before changing feature code.
The same target directory was reused. Measurements run sequentially without concurrent builds.

The shipped changes reuse canonical short `@index` strings and parsed unplaced SVG paths.
The first is a fixed table of at most 4,096 immutable tokens of at most five bytes each, plus
fixed slot/Arc metadata. It stores no resolved geometry or document-specific context. Other
strings, including noncanonical reference spellings, stay verbatim. Existing dictionary byte
and entry charges still apply even when storage is shared. A root/version regression uses
separate outline tables whose `@0` tokens mean different shapes.

The parsed-path cache admits at most 2,048 entries and 8 MiB of charged data on 64-bit targets
(2 MiB on 32-bit). Charges include actual parsed vector capacity, retained source bytes and a
128-byte per-entry allowance; entry count separately bounds map overhead. Oversized paths are
still parsed/placed without retention. Parsing happens outside the mutex. Clearing layouts
also clears parsed paths. Every glyph still clones the immutable source curve and eagerly
applies its original full affine, with the existing range, finite and work checks. No placement
is moved past the restore/open timing boundary. File/document/snapshot caps are unchanged.

The numeric byte-plane/predictor experiment regressed its isolated numeric microbenchmark and
was discarded. The shipped numeric codec, float bits and wire/native versions are unchanged.

#### Full storage/open benchmark

The unchanged `cargo xtask bench text 1000,10000` workload uses unique 117-character paragraphs,
embedded Inter Regular/Bold at 14/20 px and no images/history. One warm-up precedes five
measured samples; reported values are medians. Reopen includes all validation, full geometry
decoding/pool expansion and session initialization. Main stores no appearance records; the
feature retains all 1,170,000 glyph placements. This available-font workload does **not**
measure missing-font SVG restoration. Save includes thumbnail/native ZIP encoding in memory.

| Text layers | Run | First save ms | Repeat save ms | Reopen ms | ZIP bytes | Appearances |
|---:|---|---:|---:|---:|---:|---:|
| 1,000 | fresh main | 162.383 | 160.204 | 5.477 | 127,941 | 0 |
| 1,000 | initial before | 249.118 | 251.267 | 33.414 | 245,063 | 1,000 |
| 1,000 | main control | 159.090 | 158.854 | 5.540 | 127,941 | 0 |
| 1,000 | initial after | 245.253 | 247.934 | 30.883 | 245,063 | 1,000 |
| 1,000 | paired before | 238.612 | 243.094 | 29.591 | 245,063 | 1,000 |
| 1,000 | paired after | 240.270 | 241.112 | 29.025 | 245,063 | 1,000 |
| 1,000 | fresh main repeat | 150.825 | 146.161 | 5.245 | 127,941 | 0 |
| 10,000 | fresh main | 2852.012 | 2772.547 | 68.487 | 265,409 | 0 |
| 10,000 | initial before | 4422.316 | 4409.570 | 341.927 | 1,286,888 | 10,000 |
| 10,000 | main control | 2842.914 | 2818.195 | 69.878 | 265,409 | 0 |
| 10,000 | initial after | 4257.725 | 4325.508 | 328.378 | 1,286,888 | 10,000 |
| 10,000 | paired before | 4221.670 | 4236.802 | 330.635 | 1,286,888 | 10,000 |
| 10,000 | paired after | 4204.020 | 4059.543 | 315.444 | 1,286,888 | 10,000 |
| 10,000 | fresh main repeat | 2839.971 | 2699.317 | 64.838 | 265,409 | 0 |

The adjacent feature pair reopens in **330.635 → 315.444 ms (4.59% less time)**;
the first feature comparison is 341.927 → 328.378 ms. Both favor this change, but these are
host samples, not a cross-machine guarantee. The writer is unchanged; save-time changes are
not claimed as a new optimization. Versus the final fresh-main sample, reopen is **4.87x**,
repeat save **+50.4%**, and saved size **4.85x**. The approximately 10%
default-path acceptance target is still not met; no opt-in/capture policy changed.

#### Isolated decode and missing-font restoration

Two ignored release tests provide repeatable diagnostics:

- `cargo test -p figcraft-doc --release portable_decode_timings -- --ignored --nocapture`
  decodes 10,000 appearances of 117 glyphs (1,170,000 records), with short native-style reference
  dictionaries. Numeric timing decodes compact records alone; appearance timing also includes
  JSON metadata, bounded outline reading and final model records. These are nested/different
  workloads, not extra times to sum with the full-open benchmark.
- `cargo test -p figcraft-text --release portable_restore_timings -- --ignored --nocapture`
  restores 1,000 unique missing-font texts, 117,000 glyphs, from 32 repeated curved SVG shapes
  of 20 path elements each. Layout/parsed caches clear outside each timed batch; every placed
  curve is fully materialized and transformed before return. This directly calls restoration
  to avoid a layout-cache hit hiding the work. It is not comparable to main's substitute-font
  output and is not the available-font reopen metric.

After both test binaries are built, three before/after test-process pairs run in alternating
order without compilation. Each process reports the median of five samples after one warm-up;
the table reports the median and range of those three process medians. The earlier restore
harness's literal `curves_per_shape=24` label was corrected to the actual 20 path elements;
the generated shapes and timed workload did not change.

| Diagnostic | Before ms (range) | After ms (range) | Median change |
|---|---:|---:|---:|
| Numeric records, 10k layers | 28.765 (28.428–28.772) | 28.283 (28.104–30.417) | -1.68% |
| Complete appearance decode, 10k layers | 107.841 (105.627–110.055) | 89.137 (88.866–93.875) | -17.34% |
| Eager missing-font restore, 1k layers | 62.502 (61.711–64.006) | 38.124 (38.066–38.186) | -39.00% |

Numeric code is unchanged, so its small movement is not claimed as a speedup. The larger,
non-overlapping appearance/restore ranges support the reuse changes for these fixtures.

Before/after native reread/resave archives are byte-identical (1,286,888 bytes), including
21,222,910 JSON bytes with SHA-256
`5b0dcb8274a55acba9d03252c6114115a637382e6eb82935b2bf68e32da47ec9`.
Tests retain precise/signed-zero eager placements, current metadata/source immutability,
separate native pool scopes, invalid-input guards, independent cache byte/entry limits,
counter overflow and uncached oversized paths. Working app/server packages are untouched.
