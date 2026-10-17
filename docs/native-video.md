# Native video foundation

This bounded native-video slice adds a portable video fill, with a separate encoded source and a static poster. It is a prerequisite for Figma media import; it does not implement Figma video actions or claim Figma media parity.

## Accepted sources

The decoder accepts one silent AVC Baseline track in a nonfragmented MP4 container. Video must be progressive 8-bit 4:2:0, with constant-rate, non-reordered timestamps. The initial limits are:

| Resource | Limit |
|---|---|
| Encoded source | 16 MiB |
| Video store in one document | 32 MiB |
| Distinct video bytes across a saved archive's current state, versions, branches and library | 256 MiB |
| Dimensions | 1280 × 720 |
| Duration | 60 seconds |
| Samples | 1800 |
| Frame rate | 30 frames/second |
| Reference frames | 4 |
| Decoder picture buffer (including delayed output) | 16 |
| Distance between verified IDR frames | 60 samples |
| Cached output frames per decoder | 4 |
| Nodes considered per active-source scan, including patterns and overlays | 4096 |

Audio, additional tracks, unsupported H.264 profiles, fragmented or encrypted files, oversized sample tables, and changing SPS parameters are rejected. Audio is not silently discarded. A sample table is bounded before the container library expands it, and source and in-band SPS dimensions are checked before decoding. Decoder errors remain errors; no nearest-frame or gray-frame substitution is used.

SDR BT.601 and BT.709 matrices support both limited and full range. Explicit container color metadata takes precedence over the coded picture metadata. Unsupported matrices, transfer functions, HDR metadata and embedded ICC profiles are rejected instead of being silently rendered with the wrong conversion.

The implementation uses the public `filmcraft-h264`, `filmcraft-isobmff`, and `filmcraft-bitstream` crates at revision `2c5ba7a72619935f0481f99d59409e0d1844b234`, with default features disabled. It does not use the broad FilmCraft media hub or require FFmpeg at runtime. FFmpeg generated the synthetic test fixtures and independent pixel oracle.

## Placement and storage

`video.place` accepts a file path or base64 bytes, plus optional placement geometry. `file.placeVideo` without input requests the MP4 picker; programmatic `video.place` never opens a dialog. `file.import` routes `.mp4` names to the same bounded placement path.

Placement validates the source and decodes its first frame before acquiring document assets. The resulting `VideoPaint` references an encoded source by content hash and an ordinary `ImagePaint` poster. Crop, scale mode, rotation and image filters belong to the paint and can be edited without changing the source bytes. Placement runs on the interface thread: its work is bounded by the source size, because samples must lie inside the file without overlapping and each sample holds a limited number of NAL units, but it is not cancellable.

Video data is preserved by native save/load, local versions and branches, clipboard transfers, appended native documents and local libraries. A failed command rolls back partial document edits. Video archives use format version 2; archives with no video in any stored state retain version 1. Older readers must refuse the newer format instead of silently losing the media. A file whose video member is missing, damaged or does not match its hash opens with a warning and shows the poster in place of the video, so it can still be saved. Saving never fails because video bytes are missing: such paints are written as their poster (in the document, versions and branches). Hand-written video paints (`node.set`, styles, plugins) must name a video the document holds, and pasted properties from another file bring the poster instead. Videos that no layer paints are not written; placing a video also frees store space used by unpainted videos, while undo keeps them available.

## Rendering and sharing

The editor canvas and PNG, SVG and PDF exports render the static poster. `.fig` export also writes the poster and reports that video was flattened. Animated frames are supplied explicitly to the prototype renderer; playback time is transient and does not create document history steps.

The prototype player autoplays up to four distinct active sources, with pause, resume and restart controls. It holds each clip's final decoded frame. This first player uses a shared clock for its active clips: entering a screen or changing the active source set, including through an overlay, restarts that clock. Repeated paints of the same source share its frame. More than four active sources and missing media are reported visibly.

Placement and playback require the desktop app. The browser preserves existing video assets in native files and renders their posters; it does not bundle the new decoder. Its prototype view explains that moving video requires the desktop app.

Live sessions and online publishing of video documents are not supported in this slice. Hosting or publishing a document containing video is refused before contacting the transport. Adding video is rolled back while a host connection is pending, before its welcome, as well as after the live session is established. Use a local `.figcraft` file to transfer the complete source and poster. A live-session server also refuses any change that carries a video or a video paint, so a peer cannot add one. The same refusal covers a published snapshot and a saved session being restored.

Audio playback, a general media timeline, Figma `ON_MEDIA_HIT`, `ON_MEDIA_END` and `UPDATE_MEDIA_RUNTIME`, and verified real `.fig` video import remain separate work.

## Validation status

Model and archive regression tests cover preservation of the video source and poster through save and load, versions, branches, the clipboard and local libraries. Native desktop control checks with generated synthetic inputs covered placement, autoplay, pause, resume, restart, final-frame hold, save and reopen, and rejection of audio-bearing input. Screenshot comparisons showed changing frames during playback and identical frames while paused or at the end. The input frame rate is not a measured playback-throughput guarantee. Real-file and other native-platform runtime validation remain open.

The browser build compiles, but browser runtime open and playback were not verified. Compilation and model tests do not replace that check.
