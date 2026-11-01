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
`es-419`), French (`fr`), German (`de`), Dutch (`nl`), Brazilian Portuguese (`pt-BR`), Japanese
(`ja`), Korean (`ko`), Simplified Chinese (`zh-Hans`), Czech (`cs`), Indonesian (`id`), Finnish
(`fi`), Bengali (`bn`), Italian (`it`), Vietnamese (`vi`), Thai (`th`) and Bosnian (`bs`), machine-drafted and waiting for native speakers' review. Polish (`pl`,
Polski) was reviewed and approved by Lucas (@lukasz-sklad). Georgian (`ka`) was written with AI
assistance and reviewed by a native speaker, and stays a draft until a maintainer changes its status.
Russian (`ru`, Русский) was reviewed and approved by VanTan; its catalog keeps Frame, Dev Mode and
Smart animate as familiar tool names and uses « » quotes. Turkish (`tr`) was machine-drafted and
reviewed by a native speaker, and Swedish (`sv`) translates every UI string, command label and menu wording.

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
give only `other`. Georgian has no letter case (never write Mtavruli for capitals), uses verbal nouns
for commands (გვერდის დამატება) and keeps a noun singular after a numeral, so counts read
"შრე: {n}" and its plural entries give the same text for `one` and `other`. Vietnamese uses concise
imperatives for buttons and menu items (Lưu, Xóa) and keeps a noun unchanged after a count, so its
plural entries give only `other`. Thai is concise, with bare imperatives for buttons and
menu items (บันทึก, ลบ), no ครับ/ค่ะ endings, no spaces between Thai words (a space only separates
clauses, and Thai from Latin words or numbers), and design terms transliterated as Thai designers say
them (เฟรม, เลเยอร์, คอมโพเนนต์, อินสแตนซ์, ออโต้เลย์เอาต์). It has no plural forms: plural entries give
only `other`, and dates read `{day} {month} {year}` with Thai month abbreviations. Bosnian is
written in Latin script with ijekavian forms and Bosnian words (historija, hiljada, sačuvati,
sedmica), uses bare imperatives for buttons and menu items (Sačuvaj, Izbriši) and the formal Vi for
instructions (Odaberite, Kliknite), „ “ quotes, and consistent design terms (okvir, sloj,
komponenta, instanca, varijanta, automatski raspored, ograničenja, ispuna, obrub, efekat, varijabla,
stil, način for a variable mode, dodatak for a plugin).

CJK and Thai text need their own fonts, which egui and Inter lack: the desktop app adds an installed
one when something on screen needs it, so layer and page names in those scripts show whatever the
UI language (see `crates/ui-egui/src/cjk.rs`). The web build has no system fonts,
so Japanese, Korean and Chinese appear there only in a build with craft-fonts and
`FIGCRAFT_EMBED_CJK=1` (Noto Sans CJK SC, +16 MB); otherwise they are left out of the picker. A
Chinese UI puts its Chinese face first so Han takes Simplified Chinese shapes. Georgian is drawn in
craft-fonts' Noto Sans Georgian, one of the built-in faces in `theme::install_fonts`, or else in an
installed font on the desktop. Web builds keep it only when the craft-fonts they're built with has
it (`crates/text/build.rs`); the commit releases pin doesn't yet (`docs/releasing.md`), so the web
picker leaves Georgian out. A new CJK or other non-Latin language also needs its script in
`cjk::language`, or a built-in craft-fonts face.

## Arabic draft

`ar.toml` adds العربية as a draft: 1,857 UI entries, 620 command labels and 31 menu
wordings (2,508 entries, measured 2026-10-28). Contributed by @hussain-hz with OpenAI Codex;
not yet reviewed by a native speaker. Existing language catalogs are unchanged.

The catalog uses Modern Standard Arabic, logical Unicode order, Western digits, and Arabic
punctuation. Product names, command ids, keyboard glyphs and `{placeholders}` are preserved.
English plural entries give only `other`, with count-neutral wording such as `عدد الطبقات: {n}`.
This is deliberate: the current plural `Forms` have no zero/two fields. It is not an implementation
of Arabic's six CLDR categories; wording remains valid for 0, 1, 2 and larger integer counts.

Arabic text uses the existing bidi/shaping and Arabic system-font fallback on desktop. Web builds
require an embedded Noto Sans Arabic face to offer Arabic. The current web font budget filters
that face out, so Arabic is not offered on the web yet. No font is added to
this repository. The interface's panels are not mirrored; this catalog does not claim a complete
right-to-left interface. Native review and representative in-app checks remain required.

## Adding a language

1. Copy `crates/ui-egui/locales/es.toml` to `<code>.toml` (a BCP 47 code: `fr`, `pt-BR`, `ja`) and
   set `[meta] name` and `status = "draft — needs native review"`.
2. Translate every value; keep the keys. Keep `{placeholders}` exactly, "…" where English has it,
   brand names and keyboard glyphs. English labels of commands: `figcraft-cli commands`.
3. Plurals: give `one` only if the language has a singular form (Japanese, Korean, Chinese and
   Vietnamese give only `other`). `other` is required; `one`, `few` and `many` are optional. `trn` picks `one` for
   1 (also 0 in French and Brazilian Portuguese). Russian uses `one` for 1, 21, 31… except 11;
   `few` for 2–4, 22–24… except 12–14; and `many` for the other integers, including 0. Keep `{n}`
   in all Russian forms. Add new rules to `plural` in
   [`crates/ui-egui/src/i18n.rs`](../crates/ui-egui/src/i18n.rs). A missing form reads `other`;
   a missing key uses the fallback catalog's own plural rule. Czech uses `one` for 1, `few` for
   2–4 and `other` for 0 and 5+ (e.g. `{n} vrstva`, `{n} vrstvy`, `{n} vrstev`). Keep `{n}` in all
   three forms. Czech's CLDR `many` applies only to decimals; `trn` takes integer counts.
   Polish uses `one` for 1 only, `few` like Russian (2–4, 22–24… except 12–14) and `many` for
   the other integers, including 0 and 21 (e.g. `{n} warstwa`, `{n} warstwy`, `{n} warstw`).
   Bosnian uses `one` for 1, 21, 31… except 11, `few` for 2–4, 22–24… except 12–14, and `other`
   for the rest, including 0 (e.g. `{n} sloj`, `{n} sloja`, `{n} slojeva`); since `one` covers 21,
   keep `{n}` in every form.
4. A string added later shows English in every language until someone translates it; a missing
   translation never fails a check or a build. `cargo xtask i18n-report` lists what each language
   lacks (UI strings, command labels, menu wordings), and `cargo xtask ci` prints the same list.
5. Run the check and look at the panels in the app (longer words must fit the right panel):

       cargo test -p figcraft-ui-egui i18n

   Dropdowns, the right panel's two-column labels and the home cards cut a label that doesn't fit
   with "…" and show it in full on hover; elsewhere, look for clipped or wrapped text.

The check fails on keys the code uses that `en.toml` lacks (it prints the lines to paste), keys no
code uses, unknown command ids and placeholders that don't match English. Machine-drafted catalogs
stay marked as drafts until a native speaker has reviewed them; a reviewed catalog stays a draft
until a maintainer approves it (its `status` says which).

## Adding strings in code

Wrap every string a user sees or a screen reader hears: `tr` for plain text, `trf` for `{named}`
placeholders, `trn` for counts, `tr_noop` for literals in `const` tables that are looked up later
with `tr(label)`. The key must be a string literal as the first argument (the check reads the
source). Don't wrap ids, command ids, JSON, log messages, file extensions or document content. Then
add the key to `en.toml`; translations may follow later.

Labels from the engine and document crates (tool names, blend modes, easing curves…) are looked up
at display with `tr(x.label())`; they are listed in `dynamic_labels` in `i18n_tests.rs`.

The importers' fixed notes (the string literals passed to `warn` in `crates/format/src/fig` and
`crates/format/src/rest`) are keys too: the import report shows them in the UI language, so a new
note goes into `en.toml`. Notes built with `format!` show in English.
