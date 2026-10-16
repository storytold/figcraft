# Translating FigCraft

FigCraft's interface strings live in one TOML catalog per language in
[`crates/ui-egui/locales/`](../crates/ui-egui/locales). The build bakes every catalog into the
binary (desktop and web); nothing is read at run time. Preferences ▸ Language picks the language
and the interface switches at once. System takes the first of the OS's preferred languages that has
a catalog (the browser's language on the web; on Windows only the first preferred language counts,
since that list also holds the languages added for a keyboard, and a first language without a
catalog reads English): a regional tag reads its language's catalog (`fr-CA` → `fr`), a language
with only a regional catalog reads that one (`pt-PT` → `pt-BR`), and Latin American and US Spanish
(`es-MX`, `es-AR`, `es-CO`, `es-US`, `es-419`…) read `es-419`.
Chinese (`zh`, `zh-CN`, `zh-SG`, `zh-Hans`) reads `zh-Hans`; Traditional Chinese (`zh-Hant`,
`zh-TW`, `zh-HK`, `zh-MO`) has no catalog and reads English (or the next OS language), not
Simplified.
`pref.language {language}` does the same from the CLI, MCP and the control channel.

The catalogs: English (`en`, the source), and Spanish (`es`, with the Latin American overlay
`es-419`), French (`fr`), German (`de`), Brazilian Portuguese (`pt-BR`), Japanese (`ja`), Korean (`ko`),
Simplified Chinese (`zh-Hans`), Czech (`cs`) and Indonesian (`id`), machine-drafted and waiting for
native speakers' review. Russian (`ru`, Русский) was reviewed and approved by VanTan; its catalog
keeps Frame, Dev Mode and Smart animate as familiar tool names and uses « » quotes. Turkish (`tr`)
was machine-drafted and reviewed by a native speaker.
`es-419`), French (`fr`), German (`de`), Brazilian Portuguese (`pt-BR`), Japanese (`ja`), Korean (`ko`)
and Simplified Chinese (`zh-Hans`), machine-drafted and waiting for native speakers' review, and
Swedish (`sv`), which translates every UI string and some command labels.

## The catalogs

- **`en.toml`** lists every key. A key is the English text as the code writes it, so the code stays
  readable: `tr("Fill")`, `trf("Delete {name}?", &[("name", &name)])`, `trn("{n} layers", count, &[])`.
  Values equal their keys except plurals, which give both forms:
  `"{n} layers" = { one = "{n} layer", other = "{n} layers" }`.
- **`es.toml`** and other languages translate the same keys under `[ui]`, plus command labels by
  command id under `[commands]` (the Actions palette, the shortcuts panel and the menus) and, under
  `[menu]`, the few menu items the main menu words differently from the command (the Select matching
  submenu's "Fill" for "Select same fill"). Command ids, the CLI, MCP and control-channel output stay
  English.
- `[meta] name` is the language's own name (the picker shows it untranslated); `[meta] status` marks
  a draft (the Preferences window says so).
- A **regional overlay** (`es-419.toml`: a code with a region whose language has a catalog) lists
  only the entries worded differently there; everything else reads the language's catalog, then
  English. The check fails on an overlay entry that reads as in the catalog under it.

A key a catalog lacks shows its English text, so a partial catalog is safe to ship.

Typography follows each language: French puts a no-break space (`\u00A0` in the TOML) before
`:` `;` `?` `!`, inside « » and before `%`; German uses „ “ and formal address (Sie); Brazilian
Portuguese uses Brazilian words (arquivo, salvar, tela, você), not European ones. Latin American
Spanish overrides Spain's wording where it differs (agregar, presionar, configuración for settings,
Enter for the key, control for a gamepad, resorte) and keeps Spanish's tú. Japanese is
concise, without です/ます endings, with full-width （）「」：、。 where Japanese UI text has them;
Korean labels end without 합니다 (instructions in -세요). Simplified Chinese is concise, with
full-width （）：，、。“” in Chinese text and a space between Chinese and Latin words or numbers.
None of the three has plural forms. Turkish uses bare imperatives for buttons and menu items
(Kaydet, Sil) and the polite -in/-ın for instructions (seçin, tıklayın), puts the percent sign before
the number (%50) and keeps a noun singular after a count, so its plural entries give only `other`.
Czech uses formal address, „ “ quotes and no space before `:`. Use consistent design terms (rámec,
komponenta, instance, varianta, automatické rozvržení, omezení, výplň, tah, efekt, prototyp,
proměnná, styl). Indonesian uses bare imperatives for buttons and menu items (Simpan, Hapus), the
formal Anda (never kamu), keeps established design terms in English (frame, layer, component,
instance, auto layout, plugin, file) and keeps a noun unchanged after a count, so its plural entries
give only `other`.

CJK text needs a CJK font, which egui and Inter lack: the desktop app adds an installed one when
something on screen needs it (see `crates/ui-egui/src/cjk.rs`). The web build has no system fonts,
so Japanese, Korean and Chinese appear there only in a build with craft-fonts and
`FIGCRAFT_EMBED_CJK=1` (Noto Sans CJK SC, +16 MB); otherwise they are left out of the picker. A
Chinese UI puts its Chinese face first so Han takes Simplified Chinese shapes. A new CJK or other non-Latin
language also needs its script in `cjk::language`.

## Adding a language

1. Copy `crates/ui-egui/locales/es.toml` to `<code>.toml` (a BCP 47 code: `fr`, `pt-BR`, `ja`) and
   set `[meta] name` and `status = "draft — needs native review"`.
2. Translate every value; keep the keys. Keep `{placeholders}` exactly, "…" where English has it,
   brand names and keyboard glyphs. English labels of commands: `figcraft-cli commands`.
3. Plurals: give `one` only if the language has a singular form (Japanese, Korean and Chinese give
   only `other`). `other` is required; `one`, `few` and `many` are optional. `trn` picks `one` for
   1 (also 0 in French and Brazilian Portuguese). Russian uses `one` for 1, 21, 31… except 11;
   `few` for 2–4, 22–24… except 12–14; and `many` for the other integers, including 0. Keep `{n}`
   in all Russian forms. Add new rules to `plural` in
   [`crates/ui-egui/src/i18n.rs`](../crates/ui-egui/src/i18n.rs). A missing form reads `other`;
   a missing key uses the fallback catalog's own plural rule. Czech uses `one` for 1, `few` for
   2–4 and `other` for 0 and 5+ (e.g. `{n} vrstva`, `{n} vrstvy`, `{n} vrstev`). Keep `{n}` in all
   three forms. Czech's CLDR `many` applies only to decimals; `trn` takes integer counts.
4. To require full coverage, add the code to `COMPLETE` in
   [`crates/ui-egui/src/i18n_tests.rs`](../crates/ui-egui/src/i18n_tests.rs). Every later PR that
   adds a string then has to translate it into each complete language, so only Spanish is there:
   French, German, Brazilian Portuguese, Japanese, Korean, Simplified Chinese, Czech and Indonesian
   are registered as drafts, Russian and Turkish are reviewed but not required, and a string added
   later shows English in them until it is translated. To list what a draft lacks,
   add its code to `COMPLETE` locally and run the check.
5. Run the check and look at the panels in the app (longer words must fit the right panel):

       cargo test -p figcraft-ui-egui i18n

   Dropdowns, the right panel's two-column labels and the home cards cut a label that doesn't fit
   with "…" and show it in full on hover; elsewhere, look for clipped or wrapped text.

The check fails on keys the code uses that `en.toml` lacks (it prints the lines to paste), keys no
code uses, unknown command ids, placeholders that don't match English, and, for complete languages,
anything untranslated. Machine-drafted catalogs stay marked as drafts until a native speaker has
reviewed them.

## Adding strings in code

Wrap every string a user sees or a screen reader hears: `tr` for plain text, `trf` for `{named}`
placeholders, `trn` for counts, `tr_noop` for literals in `const` tables that are looked up later
with `tr(label)`. The key must be a string literal as the first argument (the check reads the
source). Don't wrap ids, command ids, JSON, log messages, file extensions or document content. Then
add the key to `en.toml` and to each complete language (Spanish and Russian); the drafts may follow later.

Labels from the engine and document crates (tool names, blend modes, easing curves…) are looked up
at display with `tr(x.label())`; they are listed in `dynamic_labels` in `i18n_tests.rs`.
