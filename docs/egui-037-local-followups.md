# egui 0.37 local implementation — stage 1 notes and follow-ups

Current candidate font pin: `37827adc41fd165f9a9234ac40f94dbad353aa69` restores the complete prior Latin catalog
alongside the migration fallbacks. Central font tests and license checks passed;
publication is pending approval. Managed run `egui037-figcraft-latin-ci-20261025-0711`
passed with required fonts: five targeted build-input tests plus full CI, including
WASM and plugin builds (3,257 passing executions, zero failures, nine ignored across
72 summaries; repeated tests are counted). This supersedes the optional-font-only
qualification below for the restored catalog.

## Current local qualification — 2026-10-25

Latest combined-source qualification includes main `2a3b7d4fa78e67ecdc7bdfe8b4781e50e5b2612e`, preserving its text-on-path, style-variable, export and Dev Mode changes. Managed full CI `egui037-figcraft-main2a-ci-20261025-0745` passed with required fonts: 3,297 passing executions, zero failures and nine ignored across 71 summaries, including WASM and plugin builds. Fresh independent review accepted the integration. The capture/IME/field-save refactor addresses the prior Sonar complexity findings; remote analysis still needs the approved update. Native/browser, platform IME and paired performance gaps below remain open.

### Earlier review-fix qualification

The published refresh preserves main `9f788753` and passed managed full CI
`egui037-figcraft-main-refresh-ci-20261025-0618`: 3,247 passing executions, zero
failures and nine ignored. The 3,242 counts below describe the earlier `10702311`
qualification, before five additional main-branch tests. These are execution counts,
not distinct regressions. The release font input is under review separately; a
passing optional-font host build does not qualify the required web font catalog.

### Earlier qualification at `10702311`

The migration preserves captured main `10702311f345393ee1d8868c3257679befcfa14d`
(53 commits and 114 paths beyond the first integration). The final review commit records
both the migration history and that upstream target as parents. The selected egui revision is
`6d518fc799681571963842f95b55a6b68431b15a`, winit is
`a517b3846ee8457d84ee4975110bfda6797c44dc`, and craft-window is
`25da1314d443171b4adeaad2cd9454b273f35c6f`. Cargo.lock SHA-256 is
`7ba8af4de70ba657a652266ddee2920bfc501c851520a49ac60c8044a5c23cc1`.
The normal UI enables `default_fonts` and `monochrome_emoji_fonts`.

Managed full CI `egui037-figcraft-final-ci-20261024-2312` passed with bundled fonts.
Managed full CI `egui037-figcraft-system-font-ci-20261025-0402` also passed without
bundled craft-fonts, using a licensed DejaVu Sans system-font fixture via XDG_DATA_HOME.
The final qualification run `egui037-figcraft-final-qualification-ci-20261025-0453`
repeated that configuration successfully after the documentation reconciliation.
Both runs completed formatting, strict lint, workspace tests, doctests, translations,
layering, assets, WASM checks, and plugin fmt/lint/tests/release-WASM builds. Each log
contains 3,242 passing executions, zero failures and nine ignored across 70 summaries;
this includes repeated translation checks and is not a count of distinct regressions.
The main doctest stage selects zero tests. Three plugin bundles were hash-verified and
preserved for each configuration.

The no-bundled-font run includes two test fixture corrections: text-fit cases explicitly
scan installed fonts, and a concurrent collaboration test explicitly selects its foreground
document after both joins complete instead of depending on network completion order.
Independent review accepted these changes with the original layout, synchronization and
isolation assertions intact. All 17 field-save regressions remain covered, including
owner-bound deferred saves, same-pass document replacement and later valid edits/saves.
Incoming IME interruption, screenshot geometry admission, and document/window ownership
are implemented. Historical notes below describe the earlier planning stages.

The final review packet binds source, lock, logs, plugin outputs and commit ancestry.
Native visible GUI/browser workflows, native/custom-editor IME, paired performance,
current-source platform runtime and actual release installation/signing remain open.
The system-font result is not a claim that missing document fonts are interchangeable.
Human review and human merge are required; no publication is authorized by this document.

## What this stage changes

- Dependency family at 0.37 (`egui`, `egui_extras`, `eframe`, the app's `clipboard` feature), with
  `ui-egui` enabling the production `default_fonts` fallback and tests explicitly enabling
  `default_fonts` + `monochrome_emoji_fonts` (kittest's own dev-features do not propagate to us).
- Mechanical API ports: `Id::new` → `Id::unique`, `Ui::id` → `Ui::scope_id`,
  `WidgetType` → `Role`, `MouseWheel::source`, `Shape::Band`, `ScrollArea::clamped_offset`.
- Lifecycle split: `FigApp::background_tick` (logic-safe work: control queues, live polling,
  background completions, autosave/recovery deadlines) and `FigApp::ui_setup` (fonts/language/
  theme/UI zoom/render worker, once per real frame). Direct `run_ui` harnesses tick the
  background once in `FigApp::ui` and clear the marker afterwards, so every direct frame ticks
  instead of every other one; `App::logic` ticks it for native/web. Child viewports do not repeat
  the root's background work, but each runs `FigApp::viewport_setup` in its own frame: theme and
  `ui.dark`, language, fonts (including CJK faces for its own document) and the zoom plan.
- A monotonic `FrameClock` (`web_time::Instant`) drives autosave, auto-save and capture deadlines.
  Tests inject it (`set_test_now`); UI-time (`ctx.input(i.time)`) is not used for deadlines.
- Local capture-slot adapter in `crates/ui-egui/src/control.rs` (no `craft-ui` dependency):
  weak one-shot callback, per-request identity/serial, deadline/cancel, document + viewport
  generation, capture-time geometry epoch (size/DPI/zoom only, not window position), retry on
  geometry changes, fail-closed completion when current geometry is unrepresentable, error on
  document changes, straight-RGBA PNG reply, and the 4-request / 128 MiB-pixels /
  256 MiB-aggregate budget counted over in-flight and completed-but-undrained images. The budget
  reserves the larger of the encoder phase (readback + straight-RGBA copy + PNG) and the reply
  phase (readback + PNG + base64); the encoder copy is dropped before base64 is built, and a
  post-encode reply cap refuses an oversized PNG before the base64 string is allocated. The
  request's document and window are fixed at admission and never rewritten at issuance, so a tab
  switch while settling is an error rather than the new tab's pixels. Admission and issuance happen
  in the UI phase. A hidden request, or one whose platform visibility is not reported, is parked
  with its deadline and asks for `RequestPaintWhileHidden`; a completed slot asks again for the
  real pass that drains it instead of waiting for the deadline. Settlement counts completed UI
  frames, so a discarded sizing pass never serves as a settled capture frame.
- Accessibility: the custom panel edges (`Id::unique(("panel_edge", …))`) no longer sense focus;
  `a11y::begin` records the root `Ui::scope_id` so a real `panel.__resize` ID is derivable if a panel
  ever becomes resizable. Preferences (`prefs.panel_widths`) and fixed geometry are unchanged.
- Synthetic input from the control channel: one pending batch per window, tagged with the
  document it was injected for (dropped if the active file changed), queued in the live pane of the
  window drawing now (a child window's batch is not written into the swapped-out default pane),
  applied exactly once per raw-input frame (shortcuts return on a discarded pass's rerun instead of
  inserting or running again), acknowledged only from the pass that presents it, and fenced from the
  user's system clipboard in one place: `FigApp::clipboard_set_text`/`clipboard_set_png` drop every
  copy path's write (copy/cut, copy-as-CSS/SVG/PNG/text/link, property/code/comment/join-code
  copies) while the frame's input is synthetic. A synthetic `Cut` still deletes the text-edit
  selection in the document; only its OS clipboard write is fenced. Acknowledgements belong to the
  window whose batch they acknowledge, so an empty child frame cannot error the root's. Focus-epoch fencing for
  text/IME batches remains part of the E21b nextstep.
- Incoming IME byte deletion is implemented for the custom text editor
  (`figcraft_engine::textedit::delete_surrounding_bytes`, wired from `ImeEvent::DeleteSurroundingBytes`)
  and tested for multibyte CJK, a supplementary emoji, deletion-only input and one undo step.
  Partial characters are preserved (a byte count ending inside one deletes only the whole
  characters it covers, matching the fork and egui's `surrounding_bytes_to_chars`). The selection
  itself is kept, as in stock `TextEdit`, so the paired `Commit` replaces it; only the complete
  characters before/after the selection go. A delete that finds nothing is a true no-op: it keeps
  an in-progress composition (pre-edit), the revision and the document untouched. The older
  character-counted `DeleteSurrounding` event follows the same rules and has an app-level test.

## Outgoing IME surrounding text (E21b/P7b)

Implemented for FigCraft's one custom IME producer, `shortcuts::ime_output`, against the reviewed
fork API. The descriptor now carries:

- a stable owner `(document uid, text node)` and the engine's `TextEdit::editing_session` as the
  replacement epoch (a document switch or another node changes the owner; re-entering the same
  node starts a new generation without reusing the old one);
- the tracker's viewport-scoped `context_generation`, observed exactly once per viewport pass
  after the pass's IME events and engine commands. `FigApp::observe_ime` is the single call site
  (editor passes, the home/present early returns and the first-font frame all pass through it);
  an idle `observe(None)` followed by a focused one in the same pass would drop receipts and bump
  the generation, so the early shortcut/palette idles were removed. A late `window.new` that moves
  the edited file to another window makes the old viewport observe idle for that pass;
- a bounded (≤ 4000 UTF-8 bytes) committed-text snapshot from
  `figcraft_engine::textedit::ime_surrounding`: logical model text, never `bidi::display`, with the
  UI-only pre-edit excluded, character caret/anchor converted independently to UTF-8 byte offsets
  (reversed selections stay reversed), and an explicit `Unavailable` reason when no edit/text node
  exists or the selection alone exceeds the budget. Empty text is `Available("",0,0)`.
- explicit change cause: `InputMethod` only for an accepted `Commit`/`DeleteSurrounding*` receipt
  captured immediately around that mutation (delete+commit chains are one receipt chain); plain
  typing, paste, keys, text shortcuts, undo, control-channel and AutoCorrect/list AutoFormat
  changes report `Other` and dominate the frame. No-op, failed and stale receipts earn nothing.
  When the bounded snapshot is unavailable (for example a selection wider than 4000 bytes), both
  the pass observation baseline and the pre/post receipt stamps carry the engine's document
  revision, so an accepted commit that replaces that selection still earns `InputMethod`.
- `geometry` remains the actual caret (the composition cursor while composing).

A hidden/unfocused/no-editor pass observes `None`, and closing a child window calls
`end_viewport` (now covered by a windows regression). The outgoing text is copied only as the
bounded window; the query never clones the document. Converting the caret/anchor character
indices to UTF-8 bytes now takes one character walk (both offsets) instead of two or three full
scans. Remaining cost: the caret is a character index, so a caret near the end of a huge layer is
still O(n) per query; `ime_stamp` runs the query before and after each text-affecting event, so
such a layer pays O(caret/anchor) per event and at most 4000 copied bytes. The generic edit path
also still counts characters once to clamp the caret and clones the pre-edit document for
normalisation/undo.

Tests: engine (`ime_surrounding` Unicode/reversed selection, pre-edit exclusion, empty/bounded/
too-wide selection, stable owner and session replacement, reverse-selection byte deletion with a
preserved selection, character-counted deletion, no-op composition preservation, list AutoFormat
and normalisation classification, double-click epoch stability) and UI (`ime_tests.rs`: commit vs
typing/mixed causes, transformed commit cause, bare-newline/control no-ops, byte and character
delete+commit, pre-edit, logical UTF-8, empty vs unavailable, owner replacement, double-click
inside the edit, one observation per frame, late `window.new`, discarded pass, two viewports).
These are source tests only; the parent runs the managed build/test commands.

## Web hidden paint

`RequestPaintWhileHidden` is honoured by native eframe but not by the shared web runner, which logs
an unhandled command for hidden tabs. Hidden-browser-tab `ui.screenshot`/`ui.input` therefore remain
unqualified even though the native path now queues and settles them. This is a forkfix prerequisite,
not a passing claim.

## Dependency and lock prerequisites

- The committed root `[patch.crates-io]` table pins the egui family and winit to
  immutable published fork revisions. `craft-window` is pinned to the reviewed
  craft-libs revision targeting egui 0.37; its optional `egui` feature stays disabled
  because FigCraft needs only the frame probe.
- `Cargo.lock` is committed and resolves the canonical Git sources without local
  path overrides. Use `--locked`; the earlier parent integration config is not
  required to reproduce this graph.
- Fonts remain an optional central `CRAFT_FONTS_DIR` input with native system-font
  fallback; no font binaries are added here. Required release-font qualification
  is separate from optional-font host CI. Color emoji still needs a separate
  `epaint/color_fonts` feature check.

## Qualification gaps and baseline backports/perf operations

Synthetic `ui.input` for deferred child viewports still relies on the existing per-pane queue and
`windows::take_input`; eframe invokes `App::raw_input_hook` for the root update, so injected input
into a child window is not newly qualified here.

Surface generation is not exposed by the pinned eframe/egui API, so the capture epoch covers
logical content size, DPI and zoom but cannot distinguish a same-size surface loss/recreation; that
remains a qualification gap (reported by the sibling capture review).

Remaining lifecycle item from the plan, not changed in this stage: prototype/after-delay/animated-GIF
clocks still advance from `present::show` and the GIF drawing path (UI only). They should move to a
non-drawing step called from `background_tick` so hidden playback advances once per deadline and the
visible pass never repeats it.

Unverified here: compile/tests/clippy/fmt, native X11/Wayland/Windows/macOS, browser WebGPU/WebGL2,
screen readers, hidden-window forced paint, PRIMARY/W15P, and all performance numbers. Baseline
operations to measure once the parent can build: `#8731` text tessellation, `#8736` DX12 pipeline
startup, `#8512` font discovery, `#8763`/PR `#8764` TextEdit click-away, `#8748` hidden deferred
children. None of these were applied, removed or claimed as fixed.

## Required web font catalog

Native optional-font builds retain the system-font policy. A required web build must
embed Inter Regular and SemiBold plus Noto Sans Georgian Regular after filtering;
missing required face/style coverage fails the build, regardless of which manifest supplies it. Required mode
also rejects an unset `CRAFT_FONTS_DIR`. The migration's original `b076417` catalog
lacks Inter and is not a qualified required-web input. A combined central font pin restores the prior Latin catalog while preserving the new
core fallbacks. Source Sans 3 and Source Serif 4 are excluded on every target; no font binary belongs in this repository. Required-web, required-native
and optional-font configurations remain separate validation gates.
