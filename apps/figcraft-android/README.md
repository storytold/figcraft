# FigCraft on Android

FigCraft on Android: a small `apps/` crate that hosts the existing `figcraft_ui_egui` interface in an
`android-activity` (GameActivity) window, on top of `figcraft_engine`. The desktop entry crate is not
compiled, so `rfd`, `arboard` and the macOS/Windows/Linux platform code never enter the dependency
graph.

## What the shell does

- **Entry point** — builds as `libmain.so` (`[lib] name = "main"`, `crate-type = ["cdylib"]`), loaded
  with `System.loadLibrary`; `android_main` is the usual `android-activity` entry.
- **Storage** — `AndroidApp::internal_data_path()` is the data directory (`dirs` answers nothing on Android). FigCraft's `ui.json`/`prefs.json` pair is *not* loaded or saved yet, so UI state starts from the defaults each launch.
- **File pickers** — FigCraft's `Services` is a trait whose pickers return a `DialogFuture` polled across
  frames, which suits SAF better than a blocking dialog: `src/saf_services.rs` starts the
  picker on a worker and resolves the future with a real path, asking egui for a repaint
  when it does (a cancelled picker produces no input event of its own). Opens copy the
  picked document into `<filesDir>/import/`; saves answer with a scratch path in
  `export/` and publish the finished file to the user's `ACTION_CREATE_DOCUMENT`
  destination.
- **Fonts** — Android does not expose the Unix font directories, so the CJK face from `/system/fonts`
  is appended to every family *after* FigCraft installs its theme (any earlier, and the theme's named
  families would be dropped).
- **Touch** — hit targets are raised to 40 pt, sliders widened and hover tooltips suppressed, applied
  after the theme's style exists. The layout is not zoomed: it is designed for a desktop window and a
  phone gives ~411 pt, so the next step is a touch rework, not a scale factor.

## Known gaps

- Printing has no backend, so the Print dialog is unreachable rather than broken.
- Documents open from the app's own import copy; "save back to the original" is not wired.
- The collaboration client has no Android networking story yet (out of scope here).

- The system clipboard is not wired (`arboard` has no Android backend); FigCraft's own clipboard is
  still used.
- The launcher icons are the Android Studio template's placeholder art.

## Build

```bash
cargo ndk -t arm64-v8a -o apps/figcraft-android/jniLibs build --release -p figcraft-android
cd apps/figcraft-android/android && ./gradlew assembleRelease
```

`jniLibs/` and the Gradle output are gitignored; the APK is not part of this change.
