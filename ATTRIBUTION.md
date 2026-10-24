# Attribution

Every asset bundled in this repository (images, icons, fonts, data files) is listed here with its
author, source and licence. `cargo xtask assets` fails if a file under `assets/`, `docs/brand/` or
`docs/images/` is missing from this list.

FigCraft's user-interface icons are drawn in code (`crates/ui-egui/src/icons.rs`): original work,
MIT OR Apache-2.0. No third-party icon sets are used.

## Assets in this repository

| Path | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `assets/app-icon/` (all files: `figcraft.svg`, `figcraft-small.svg`, `figcraft-*.png`, `figcraft.ico`, `hicolor/`) | The project owner (engraving-style drawing, vectorised) | Original work, see [`assets/app-icon/README.md`](assets/app-icon/README.md); derived files rendered by `cargo xtask icon` | MIT OR Apache-2.0, [`assets/app-icon/LICENSE.txt`](assets/app-icon/LICENSE.txt) | FigCraft app icon (chameleon) |
| `docs/brand/artcraft-logo.svg`, `docs/brand/artcraft-logo.png`, `docs/brand/artcraft-logo-white.svg`, `docs/brand/artcraft-logo-white.png`, `docs/brand/artcraft-mark.svg`, `docs/brand/artcraft-mark.png`, `docs/brand/artcraft-mark-black.svg`, `docs/brand/artcraft-mark-black.png` | ArtCraft Team | storytold/craftrules `assets/brand/` | ArtCraft brand terms (`docs/brand/LICENSE-brand.txt`), not open source | Trademarks; unmodified use within FigCraft only |
| `docs/images/` (all screenshots and exports: `shot-*.png`, `art-*.png`) | FigCraft contributors | Screenshots of FigCraft showing synthetic demo documents made in FigCraft, and PNG exports of those documents by FigCraft's renderer | MIT OR Apache-2.0 | No third-party artwork |
| `examples/*.figcraft` | FigCraft contributors | Synthetic demo documents built through FigCraft's command API; invented copy, shapes and vectors drawn from scratch, no images or embedded fonts | MIT OR Apache-2.0 | Text uses Inter (OFL, from craft-fonts or the system); no third-party artwork |

## Fonts (build input, not in this repository)

Release builds embed fonts from the optional craft-fonts build input
([storytold/craft-fonts](https://github.com/storytold/craft-fonts), `CRAFT_FONTS_DIR`). Its own
`ATTRIBUTION.md` and licence files are authoritative; the packages ship each font's licence.

| Family | Licence | Upstream |
|---|---|---|
| Inter | SIL Open Font License 1.1 | https://github.com/rsms/inter |
| Archivo | SIL Open Font License 1.1 | [Google Fonts / Archivo](https://github.com/google/fonts/tree/6c70c829f09ea345d3590406693220ea35c6553f/ofl/archivo) |
| Roboto, Montserrat, Lato, Open Sans, Poppins, Nunito Sans, Merriweather, Playfair Display, Source Sans 3, Source Serif 4, Carlito, Caladea | SIL Open Font License 1.1 | Google Fonts / upstream repositories (see craft-fonts) |
| Liberation Sans, Serif, Mono | SIL Open Font License 1.1 | https://github.com/liberationfonts |
| BIZ UDPGothic, BIZ UDMincho, Shippori Mincho, Noto Sans Arabic | SIL Open Font License 1.1 | Google Fonts (see craft-fonts) |

egui's default fonts (compiled into the `egui`/`epaint` crates) keep their own licences (OFL / Apache-2.0 / Ubuntu Font Licence); see those crates.

## Android launcher icons (figcraft)

`apps/figcraft-android` ships the Android Studio new-project template's default launcher art, unmodified.
The adaptive icon in `res/mipmap-anydpi-v26/` composes the foreground drawable with the template's
background colour; the density folders hold the same stand-in at each scale.

| Asset | Author | Source | Licence |
|---|---|---|---|
| `apps/figcraft-android/android/app/src/main/res/drawable/ic_launcher_foreground.png` | The Android Open Source Project | Android Studio new-project template (default launcher art), unmodified | Apache-2.0 |
| `apps/figcraft-android/android/app/src/main/res/mipmap-hdpi/ic_launcher.png` | The Android Open Source Project | Android Studio new-project template (default launcher art), unmodified | Apache-2.0 |
| `apps/figcraft-android/android/app/src/main/res/mipmap-hdpi/ic_launcher_round.png` | The Android Open Source Project | Android Studio new-project template (default launcher art), unmodified | Apache-2.0 |
| `apps/figcraft-android/android/app/src/main/res/mipmap-mdpi/ic_launcher.png` | The Android Open Source Project | Android Studio new-project template (default launcher art), unmodified | Apache-2.0 |
| `apps/figcraft-android/android/app/src/main/res/mipmap-mdpi/ic_launcher_round.png` | The Android Open Source Project | Android Studio new-project template (default launcher art), unmodified | Apache-2.0 |
| `apps/figcraft-android/android/app/src/main/res/mipmap-xhdpi/ic_launcher.png` | The Android Open Source Project | Android Studio new-project template (default launcher art), unmodified | Apache-2.0 |
| `apps/figcraft-android/android/app/src/main/res/mipmap-xhdpi/ic_launcher_round.png` | The Android Open Source Project | Android Studio new-project template (default launcher art), unmodified | Apache-2.0 |
| `apps/figcraft-android/android/app/src/main/res/mipmap-xxhdpi/ic_launcher.png` | The Android Open Source Project | Android Studio new-project template (default launcher art), unmodified | Apache-2.0 |
| `apps/figcraft-android/android/app/src/main/res/mipmap-xxhdpi/ic_launcher_round.png` | The Android Open Source Project | Android Studio new-project template (default launcher art), unmodified | Apache-2.0 |
| `apps/figcraft-android/android/app/src/main/res/mipmap-xxxhdpi/ic_launcher.png` | The Android Open Source Project | Android Studio new-project template (default launcher art), unmodified | Apache-2.0 |
| `apps/figcraft-android/android/app/src/main/res/mipmap-xxxhdpi/ic_launcher_round.png` | The Android Open Source Project | Android Studio new-project template (default launcher art), unmodified | Apache-2.0 |
