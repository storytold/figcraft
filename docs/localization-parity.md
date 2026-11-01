# Localization parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-29 · **Change:** minor (Bosnian draft catalog, one/few/other plurals; German terminology corrections (M10): gradient stops, OpenType stylistic sets and the Home-screen preference; partial review, catalog remains a draft; Arabic draft catalog, count-neutral plurals and web font gate; Thai draft catalog and Thai UI fonts; a missing translation fails no check: `cargo xtask i18n-report` lists them; Vietnamese draft catalog; string counts re-measured: 2,352 entries; Russian and Turkish catalogs reviewed by native speakers, Russian one/few/many plurals; Czech and Indonesian drafts; the Japanese draft completed; counts re-measured; Windows UI language from the first preferred language; draft catalogs catch up: every draft translates every string again; Simplified Chinese draft catalog; macOS finds its downloadable CJK system fonts (#406); string counts re-measured: 2,272 entries, the drafts at 95–99%; Swedish command labels and menu wordings; Russian, Swedish, Turkish, Czech and Indonesian listed; a duplicated paragraph removed) · **Target:** Figma Design (desktop app 126.10.7)

FigCraft's interface has a localization layer with an English catalog, a Russian catalog reviewed
by a native speaker, and Spanish (with a Latin American overlay), French, German, Brazilian
Portuguese, Japanese, Korean, Simplified Chinese, Indonesian, Vietnamese, Bosnian and Czech drafts (machine-drafted,
waiting for native speakers' review), plus Swedish and Turkish catalogs. This page lists the twelve key
languages, then the other languages Figma ships. How to add a
language: [`translating.md`](translating.md).

## What Figma ships

Figma's help center lists its web, desktop and mobile apps in **English, Japanese, Korean, French,
German, Spanish (Spain and Latin America) and Portuguese (Brazil)** (checked 2026-10-10). Simplified
Chinese, Hindi, Arabic, Indonesian and Vietnamese are not offered; Figma points users to the
browser's page translation. The `.lproj` folders in `/Applications/Figma.app` (60+) are the
Electron/Chromium locale packs, not Figma's own UI languages. Text *content* in any script is
supported by both apps (CJK, Arabic and Devanagari shaping, IME); Figma has no vertical text.

## What FigCraft has (measured 2026-10-22, from source)

- **Layer:** one TOML catalog per language in `crates/ui-egui/locales/`, baked into the binary by
  the UI crate's build script (desktop and web; nothing read at run time). Keys are the English text
  (`tr("Fill")`, `trf` for `{named}` placeholders, `trn` for counts with one/few/many/other plural forms), so
  a missing translation shows English, never a raw key. One English text that needs two
  translations takes a context (`"Fill|sizing"`).
- **Strings:** 1,747 UI keys in `en.toml` (42 with plural forms; ~110 are labels from engine and
  document data, such as tool names, blend modes and Dev Mode properties) plus 574 command labels
  and 31 menu wordings by command id: 2,352 entries (counted against `en.toml` and the complete
  Spanish catalog's command and menu tables). Command ids, the CLI, MCP and the control
  channel stay English. Accessibility names come from the same strings.
- **Language preference:** Preferences ▸ Language (System, then each catalog by its own name:
  English, Bosanski, Čeština, Deutsch, Español, Español (Latinoamérica), Français, Bahasa Indonesia, 日本語,
  한국어, Português (Brasil), Русский, Svenska, Türkçe, Tiếng Việt, 简体中文), applied at once without a
  restart; System takes the first of the OS's preferred languages that has a catalog (on Windows
  only the first preferred language counts, since that list also holds the languages added for a
  keyboard; macOS AppleLanguages, `LANGUAGE` then the locale variables on Linux and BSD; a regional
  tag falls back to its language, pt-PT or pt to pt-BR, and es-419, es-MX, es-AR, es-CO and the
  other Latin American and US Spanish tags to es-419; regional ru, tr, cs and id tags (ru-RU,
  tr-TR, cs-CZ, id-ID) to their catalogs; zh, zh-CN, zh-SG and zh-Hans read zh-Hans,
  while Traditional Chinese (zh-Hant, zh-TW, zh-HK, zh-MO) has no catalog and reads English or the
  next OS language, never Simplified) or the browser's language on the web.
  `pref.language {language}` sets it from the CLI, MCP and the control channel.
- **Regional overlays:** a regional catalog whose language has a catalog lists only what differs
  (`es-419.toml` over `es.toml`); its UI strings, command labels and menu wordings read the
  overlay, then the base language, then English.
- **Check:** `cargo test -p figcraft-ui-egui i18n` fails on keys the code uses that `en.toml`
  lacks, keys no code uses, unknown command ids, placeholders that differ from English and overlay
  entries that read as in their base language. No language is held to full coverage: a string
  added later shows English until someone translates it, and `cargo xtask i18n-report` (part of
  `cargo xtask ci`) lists what each language lacks. Every draft translated every string on
  2026-10-12; by 2026-10-17 the strings added since left them at 95–99% (table below).
- **Not yet localized:** messages the engine writes (errors, a few toasts, Dev Mode property values
  such as "inside"), number formatting (decimal points stay `.`), and decimal plural rules
  (`trn` takes integers: Czech and Bosnian use one/few/other and Russian one/few/many; French and Brazilian
  Portuguese take the singular for 0 as well; Japanese, Korean, Chinese and Vietnamese have one form;
  CLDR's "many" for millions in French, Spanish and Portuguese reads as the plural).
- **Script support in documents:** Unicode bidi with Arabic shaping and mirrored brackets in text
  layers; right-to-left list markers and indents; right-to-left names in the layers panel, tabs
  and canvas labels (visual order, joined Arabic); inline IME composition with the candidate window
  at the caret; system-font fallback for scripts the UI fonts lack. Devanagari shapes through
  harfrust. No vertical text (Figma has none either).
- **Script support in the UI:** menus widen to their longest label; dropdowns, the two-column
  labels of the right panel and the home screen's cards keep a long translation on one line with an
  ellipsis and show it in full on hover (checked in German, the longest); no mirrored
  (right-to-left) layout of panels. Cyrillic, Turkish and Czech letters draw from the UI fonts
  (no font added; Russian checked in screenshots). CJK translations are shorter than English; the same ellipsis
  and tooltip cover the few that don't fit (the sizing dropdowns).
- **CJK UI fonts:** egui's fonts and Inter have no CJK glyphs. Once the UI language, the open
  file's names or comments, typed text or the language picker needs Japanese kana, Hangul or Han,
  the desktop app adds one installed face per script to the UI's fallbacks (Windows: Yu Gothic UI,
  Meiryo, Malgun Gothic, Microsoft YaHei; macOS: Hiragino Sans, Apple SD Gothic Neo, PingFang SC,
  Hiragino Sans GB, Heiti SC, also from the system's downloaded font assets
  (`/System/Library/AssetsV2/com_apple_MobileAsset_Font*`, where current versions keep PingFang);
  Linux and BSD: Noto Sans CJK, Source Han Sans, Nanum, WenQuanYi; the craft-fonts build's BIZ
  UDPGothic first), the UI language's script first; Han without kana outside a Japanese or Korean UI
  reads as Chinese, and a Chinese UI puts its Chinese face (Microsoft YaHei, PingFang SC, Noto Sans
  CJK SC) ahead of the built-in Japanese one, so Han takes Simplified Chinese shapes; changing the
  UI language lays the faces out again. Only that face's file is read and egui shares its bytes, so English users load
  nothing (startup and memory measured unchanged); no CJK font installed means tofu, never a
  crash. The web build has no system fonts: built with craft-fonts and `FIGCRAFT_EMBED_CJK=1` it
  embeds Noto Sans CJK SC (+16 MB) and offers Japanese, Korean and Simplified Chinese; otherwise the language picker and
  System leave them out. No font file is in this repo.

## Languages

| Language | Code | UI strings translated | Dialogs / tooltips / help | Script support | Native review | Status | Hours to full |
|---|---|---|---|---|---|---|---|
| English | en | 2,352 / 2,352 (100%, source) | yes | Latin | yes (source) | full | 0 |
| Simplified Chinese (Mandarin) | zh-Hans | 2,216 / 2,352 (94.2%, draft) | yes | CJK text and IME in documents; UI text in an installed Chinese face, ahead of Japanese ones (web: the `FIGCRAFT_EMBED_CJK` build); no Traditional Chinese catalog (zh-TW, zh-HK read English) | no | draft | 1–2 (after native review) |
| Spanish | es, es-419 | 2,352 / 2,352 (100%, draft); es-419 overlays 104 Latin American wordings (draft) | yes | Latin | no | draft | 0–1 (after native review) |
| Hindi | hi | 0 (0%) | no | Devanagari shaping works in documents; UI font fallback to check | no | none | 4–7 |
| Arabic | ar | 1,857 / 1,857 UI strings, 620 command labels, 31 menu wordings (draft, counted 2026-10-28) | catalog complete; native review pending | Existing bidi/shaping and Arabic system fallback; web not offered (font budget excludes Arabic); panels do not mirror | no | draft | review + panel mirroring (not re-estimated) |
| French | fr | 2,328 / 2,352 (99.0%, draft) | yes | Latin | no | draft | 1–2 (after native review) |
| Portuguese | pt-BR | 2,331 / 2,352 (99.1%, draft) | yes | Latin | no | draft | 1–2 (after native review) |
| Indonesian | id | 2,292 / 2,352 (97.4%, draft) | yes | Latin | no | draft | 1–2 (after native review) |
| Japanese | ja | 2,304 / 2,352 (98.0%, draft) | yes | CJK text and IME in documents; UI text in an installed Japanese face (web: the `FIGCRAFT_EMBED_CJK` build) | no | draft | 1–2 (after native review) |
| German | de | 2,226 / 2,352 (94.6%, draft) | yes | Latin; long words elide in narrow fields | no | draft | 1–2 (after native review) |
| Korean | ko | 2,216 / 2,352 (94.2%, draft) | yes | Hangul text and IME in documents; UI text in an installed Korean face (web: the `FIGCRAFT_EMBED_CJK` build) | no | draft | 1–2 (after native review) |
| Vietnamese | vi | 2,352 / 2,352 (100%, draft) | yes | Latin with stacked diacritics | no | draft | 1–2 (after native review) |
| Thai | th | 1,791 / 1,791 UI strings, 589 of 598 command labels, 31 menu wordings (draft) | yes | Thai text in documents; UI text (and layer and page names in Thai, whatever the UI language) in an installed Thai face, desktop only; the web build leaves Thai out | no | draft | 1–2 (after native review) |

Other languages (measured 2026-10-22): Russian (`ru`) 2,352 / 2,352 (100%), reviewed and approved
by VanTan, one/few/many integer plurals; Swedish (`sv`) 2,314 / 2,352 (98.4%), its command labels and menu
wordings added 2026-10-17 (#599), native review welcome; Turkish (`tr`) 2,291 / 2,352 (97.4%), whose header
says it was machine-drafted and reviewed by a native speaker; Czech (`cs`) 2,292 / 2,352 (97.4%), a draft
with one/few/other plurals; Bosnian (`bs`, Bosanski, added 2026-10-30) 2,566 / 2,566 (100%: 1,906 UI
strings, 629 command labels, 31 menu wordings), a machine draft awaiting native review, with
one/few/other plurals (21 takes one). Figma ships none beyond the eight above.

The layer is done. Per-language hours above are agent hours for a catalog of ~1,800 entries
(a machine draft, then a pass over the panels for overflow), plus each language's plural rule where
it isn't one/other; every language also needs a native speaker's review (human). Left before full
parity besides the languages: localizing engine messages and number formatting (4–6 h);
right-to-left mirroring of the panels is in Arabic's hours. Total: **29–51 h** (kept on
2026-10-17: Indonesian's draft takes 2–3 h off, about what bringing the drafts back from 95% to
every string adds).

Priority: native reviews of the drafts and the es-419 overlay (every one of Figma's languages
and variants has one, plus Simplified Chinese, Indonesian, Vietnamese and Czech, which Figma doesn't ship), then
Hindi and Arabic. Traditional Chinese is not planned (no catalog; its users see English).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-30 | minor | Bosnian (`bs`, Bosanski) draft catalog: every UI string (1,906), command label (629) and menu wording (31), machine-drafted in Latin script with ijekavian forms and marked as a draft awaiting native review. Bosnian plurals: one for 1, 21, 31… (not 11), few for 2–4, 22–24… (not 12–14), other for the rest. Grade and hours unchanged (Bosnian is not one of the twelve key languages) |
| 2026-10-29 | minor | German terminology corrections (M10): gradient stops, OpenType stylistic sets and the Home-screen preference; partial review, catalog remains a draft. Nine existing translations corrected; keys, placeholders, coverage, grade and estimates unchanged |
| 2026-10-28 | minor | Arabic draft: 1,857 UI entries, 620 command labels and 31 menu wordings, with count-neutral plurals. AI-assisted, native review pending. Desktop uses the existing Arabic fallback and bidi; web requires an embedded Arabic face. No panel mirroring or parity-grade increase claimed |
| 2026-10-24 | minor | Thai (`th`) draft catalog: `th.toml` translates every UI string (1,791), 589 command labels and the 31 menu wordings, machine-drafted and marked as a draft awaiting native review (9 newer command labels read English, as in the other drafts). The desktop UI now adds an installed Thai face (Noto Sans Thai, Leelawadee UI, Tahoma, Sukhumvit Set, Thonburi, Ayuthaya, Sarabun, Loma, Garuda or Waree) when a layer or page name or the UI language needs it, so Thai names no longer show as tofu squares. No plural forms for Thai. Grade and hours unchanged |
| 2026-10-24 | minor | Vietnamese (`vi`) draft catalog (#1026), assessed as a machine draft awaiting review like the others; string counts re-measured: 2,352 entries. |
| 2026-10-23 | minor | A missing translation fails no check: Spanish is no longer held to full coverage, and the tests of one language's wording, names and plural forms are gone (the plural rules and language-tag tests stay). `cargo xtask i18n-report`, run by `cargo xtask ci`, lists each language's missing UI strings, command labels and menu wordings. Grade unchanged |
| 2026-10-15 | minor | Community catalogs (#257, #263, #469, #477, #288): Russian (`ru`, Русский, reviewed and approved by VanTan) and Turkish (`tr`, Türkçe, reviewed by a native speaker), Czech (`cs`, Čeština) and Indonesian (`id`, Bahasa Indonesia) drafts, and the Japanese draft caught up; Preferences ▸ Language lists them and System maps their regional tags. Catalogs may give few and many plural forms; Russian picks one, few or many by its integer rule (11–14 take many), the catalog supplying the text choosing the rule. Only Spanish is held to completeness. Counts re-measured: 1,587 UI keys (38 plural), 559 command labels and 31 menu wordings, 2,177 entries; Spanish 100%, Japanese 2,176, Czech and Indonesian 2,166, Russian and Turkish 2,165, the five older drafts 2,088–2,089 (95%). Localization 39% and 29–51 h unchanged at this rounding |
| 2026-10-17 | minor | Chinese showed as boxes on macOS (#406): the font scan also walks the system's downloaded font assets (`/System/Library/AssetsV2` and the older `/System/Library/Assets`, only their `com_apple_MobileAsset_Font*` folders), where current versions keep PingFang SC, and Hiragino Sans GB and Heiti SC join the Chinese UI faces and the text fallbacks. Tested with a folder laid out the same way; not yet run on a Mac |
| 2026-10-17 | minor | String counts re-measured from the catalogs: 1,680 UI keys, 561 command labels and 31 menu wordings (2,272 entries); Spanish complete, the other drafts at 95.3–99.2% (strings added since 2026-10-12 show English), es-419 101 entries. Indonesian has a draft (98.8%; it was listed as none); Russian (98.8%, reviewed), Swedish (99.6%, command labels and menu wordings from #599), Turkish (98.7%) and Czech (98.8%) listed with counts; the language picker's list updated; a paragraph repeated by a merge removed. Grade and hours unchanged (Indonesian's draft is offset by the drafts' new gaps) |
| 2026-10-14 | minor | Windows UI language (#291): the interface language comes from Windows' first preferred language alone; that list is also the input-language list (a German keyboard on a Polish system adds Deutsch), and taking the first entry with a catalog turned the interface German there. A first preferred language without a catalog reads English; macOS AppleLanguages and gettext's `LANGUAGE` still take the first language with a catalog |
| 2026-10-12 | minor | Drafts catch up again, and the counts with them: French, German, Brazilian Portuguese, Japanese, Korean and Simplified Chinese translate the 71 UI strings and 5 command labels they lacked since sharing with a join code and open hosting (the Share popover, join codes, owner tokens, reports, the .fig export notes), and with Spanish the 26 strings and 3 command labels of your online files (Home ▸ Online, its menu and dialogs, the marks on tabs); machine-drafted in each file's conventions, still drafts. The Latin American overlay adds computadora and agrega where the new Spanish says ordenador and añade (98 entries). 2,076 entries per catalog (1,503 UI strings, 544 command labels, 29 menu wordings) |
| 2026-10-12 | minor | Draft catalogs catch up: French, German, Brazilian Portuguese, Japanese and Korean translate the 66 UI strings and 17 command labels added since they were drafted (cursor chat and spotlight, the plugin directory, Rename layers, presentation and snapping keys, list levels, the rotation origin, placing with Enter…), Simplified Chinese the 46 and 12 it lacked; machine-drafted from the English in each file's conventions, still drafts (not in `COMPLETE`). The Latin American overlay adds Control for a gamepad controller and presiona in the plugin directory hint (90 entries, with Enter for the keycap). String counts re-measured: 1,417 UI keys, 536 command labels and 29 menu wordings (1,982 entries), every draft at 100%. Localization grade unchanged |
| 2026-10-12 | minor | Simplified Chinese draft catalog (gap 5): `zh-Hans.toml` translates every UI string (1,371 keys; plurals give only `other`, and `plural_one` never picks "one"), command label (524) and menu wording (29), machine-drafted in a concise UI style with full-width punctuation and marked as a draft awaiting native review; Preferences ▸ Language lists 简体中文, System maps zh, zh-CN, zh-SG and zh-Hans to it, and Traditional Chinese (zh-Hant, zh-TW, zh-HK, zh-MO) has no catalog and stays English (or the next OS language), never Simplified. A Chinese UI draws Han in a Chinese face first (Microsoft YaHei, PingFang SC, Noto Sans CJK SC), ahead of the craft-fonts build's Japanese BIZ UDPGothic, and changing the UI language lays the CJK faces out again (switching from Japanese to Chinese no longer keeps the Japanese face in front). String counts re-measured: 1,371 UI keys, 524 command labels and 29 menu wordings (1,924 entries); the French, German, Brazilian Portuguese, Japanese and Korean drafts have 1,899 of them (the 25 newer strings read English). Localization 36% → 39%, 31–54 h → 29–51 h |
| 2026-10-12 | minor | Latin American Spanish overlay (gap 5): `es-419.toml` lists only the 87 entries (61 UI strings, 26 command labels) whose wording differs in Latin American usage (agregar for añadir, presionar for pulsar, configuración for settings, Enter for the Intro key, control for a gamepad, resorte for a spring) and reads `es.toml`, then English, for the rest (a regional catalog overlays its language's); same tú register as Spanish; a draft awaiting native review. Preferences ▸ Language lists Español (Latinoamérica); System maps es-419, es-MX, es-AR, es-CO and the other Latin American and US Spanish tags to it, es-ES and es to Spanish. A regional catalog's lookups (UI strings, command labels, menu wordings) go overlay → base language → English; the check fails on an overlay entry that reads as in its base. Localization 35% → 36%, 32–55 h → 31–54 h |
| 2026-10-12 | minor | Japanese and Korean draft catalogs and CJK UI fonts (gap 5): `ja.toml` and `ko.toml` translate every UI string (1,351 keys; plurals give only `other`, and `plural_one` never picks "one" in either), command label (519) and menu wording (29), machine-drafted and marked as drafts awaiting native review; Preferences ▸ Language lists 日本語 and 한국어, System picks ja* and ko*. CJK UI text renders: once the UI language, the open file's names or comments, typed text or the language picker needs Japanese kana, Hangul or Han, the desktop app appends one installed face per script to egui's fallbacks (Windows Yu Gothic UI / Meiryo / Malgun Gothic / Microsoft YaHei, macOS Hiragino Sans / Apple SD Gothic Neo / PingFang SC, Linux Noto Sans CJK; the craft-fonts build's BIZ UDPGothic first), reading only that face's file and sharing its bytes with egui (English users load none: startup and memory unchanged); the web shows Japanese and Korean only when built with craft-fonts' Noto Sans CJK (`FIGCRAFT_EMBED_CJK`), else it keeps them out of the picker and System. Localization 30% → 35%, 40–65 h → 32–55 h |
| 2026-10-12 | minor | French, German and Brazilian Portuguese draft catalogs (gap 5): `fr.toml`, `de.toml` and `pt-BR.toml` translate every UI string (1,351 keys, 36 with plural forms), command label (519) and menu wording (29), machine-drafted and marked as drafts awaiting native review; they fall back to English per key, so new strings need only English and Spanish. French and Brazilian Portuguese count 0 as singular (`plural_one`); System picks the first OS language with a catalog (Windows' and macOS's preferred-language lists, `LANGUAGE` then the locale on Linux and BSD; pt-PT and pt read pt-BR); Preferences ▸ Language lists Français, Deutsch and Português (Brasil). Dropdowns, two-column labels and the home cards keep long translations on one line with an ellipsis and show them in full on hover. Localization 20% → 30%, 45–75 h → 40–65 h |
| 2026-10-11 | major | Localization layer landed (gap 5): catalogs per language baked in at build time, English text as keys and fallback, `{named}` placeholders and one/other plurals, command labels by command id, Preferences ▸ Language (System follows the OS or the browser) applied live, `pref.language`, and a catalog check; English catalog (1,280 UI keys) and a complete Spanish draft (1,810 entries with command labels) waiting for native review; localization 10% → 20%, 60–100 h → 45–75 h |
| 2026-10-10 | major | Created: English only, string counts measured from source, Figma's eight UI languages from its help center |
