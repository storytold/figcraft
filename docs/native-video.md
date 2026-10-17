# Native video foundation

This bounded native-video slice adds a portable video fill with a separate encoded source and static poster, native audio, per-video playback options, and prototype media actions and triggers. Documented Figma media interactions can be mapped from REST data. Binary `.fig` video import and full Figma media parity remain separate work.

## Accepted sources

The decoder accepts one AVC Baseline or Main track in a nonfragmented MP4 container, optionally with one AAC-LC audio track. Video must be progressive 8-bit 4:2:0, with constant-rate, non-reordered timestamps. CAVLC and CABAC are supported. The limits are:

| Resource | Limit |
|---|---|
| Encoded source | 16 MiB |
| Video store in one document | 32 MiB |
| Distinct video bytes across a saved archive's current state, versions, branches and library | 256 MiB |
| Visible dimensions | 1920 × 1080 |
| Coded dimensions, including macroblock padding | 1920 × 1088 |
| Duration | 60 seconds |
| Samples | 1800 |
| Frame rate | 30 frames/second |
| Reference frames | 4 |
| Decoder picture buffer (including delayed output) | 16 |
| Distance between verified IDR frames | 60 samples |
| Cached output frames per decoder | 4 |
| Nodes considered per active-source scan, including patterns and overlays | 4096 |
| Independently playing video occurrences | 16 |
| Distinct active encoded sources and cached PCM clips | 4 |
| Media events dispatched in one UI tick | 64 |
| AAC channels | 1 or 2 |
| AAC sample rate | At most 48 kHz |
| AAC access units | 3000 |
| Aggregate audio/video sample-table entries | 4800 |
| Decoded PCM per source | 24 MiB |

Additional video/audio tracks, unsupported codecs and H.264 profiles, fragmented or encrypted files, oversized sample tables, and changing SPS parameters are rejected. Audio is not silently discarded. AAC decoding supports bounded leading silence and a single normal media edit for priming/trim; unsupported edit sequences are rejected. Sample tables are bounded before the container library expands them, and source and in-band SPS dimensions are checked before decoding. Decoder errors remain errors; no nearest-frame or gray-frame substitution is used.

SDR BT.601 and BT.709 matrices support both limited and full range. Explicit container color metadata takes precedence over the coded picture metadata. Unsupported matrices, transfer functions, HDR metadata and embedded ICC profiles are rejected instead of being silently rendered with the wrong conversion.

The implementation uses the public `filmcraft-h264`, `filmcraft-aac`, `filmcraft-isobmff`, and `filmcraft-bitstream` crates at revision `2c5ba7a72619935f0481f99d59409e0d1844b234`, with default features disabled. The desktop app uses CPAL 0.16 for audio output through the UI's platform-service interface. It does not use the broad FilmCraft media hub or require FFmpeg at runtime. FFmpeg generated the synthetic test fixtures and independent pixel/PCM oracles.

## Placement and storage

`video.place` accepts a file path or base64 bytes, plus optional placement geometry. `file.placeVideo` without input requests the MP4 picker; programmatic `video.place` never opens a dialog. `file.import` routes `.mp4` names to the same bounded placement path.

Placement validates the source, including audio metadata, and decodes its first video frame before acquiring document assets. Its result includes `hasAudio`. The resulting `VideoPaint` references the complete encoded source, including its audio track, by content hash and an ordinary `ImagePaint` poster. Crop, scale mode, rotation and image filters belong to the paint and can be edited without changing the source bytes. Placement runs on the interface thread: its work is bounded by the source size, because samples must lie inside the file without overlapping and each sample holds a limited number of NAL units, but it is not cancellable.

Video data is preserved by native save/load, local versions and branches, clipboard transfers, appended native documents and local libraries. A failed command rolls back partial document edits. Video archives use format version 2; archives with no video in any stored state retain version 1. Older readers must refuse the newer format instead of silently losing the media. A file whose video member is missing, damaged or does not match its hash opens with a warning and shows the poster in place of the video, so it can still be saved. Saving never fails because video bytes are missing: such paints are written as their poster (in the document, versions and branches). Hand-written video paints (`node.set`, styles, plugins) must name a video the document holds, and pasted properties from another file bring the poster instead. Videos that no layer paints are not written; placing a video also frees store space used by unpainted videos, while undo keeps them available.

## Rendering and sharing

The editor canvas and PNG, SVG and PDF exports render the static poster. `.fig` export also writes the poster and reports that video was flattened. Animated frames are supplied explicitly to the prototype renderer; playback time is transient and does not create document history steps.

Each video paint stores Autoplay, Loop and Sound options, editable in the Prototype panel. Older native video paints default to autoplay on, loop off and sound on; these are native compatibility defaults, not verified defaults for missing Figma source fields. Changing an option is undoable and preserves the original source bytes and poster.

The player keeps independent positions, play states, mute and loop settings for repeated uses of the same source. A video occurrence is identified by its node, paint index, rendered text-style batch or pattern-consumer path, and presentation scope. Two overlays of the same frame have separate scopes. Repeated tiles in one pattern cell share an occurrence; different pattern consumers do not. Text runs already combined by the text renderer into one equivalent style batch share that batch's occurrence.

At most 16 occurrences of four distinct sources are active. The scan also has a 4096-node budget; exceeding these limits or encountering missing media produces a visible diagnostic and poster fallback. Source decoders and PCM stay shared by content hash. A non-looping video holds its final decoded picture, including while a longer audio track finishes. Global pause/resume preserves individually paused videos. Entering a different screen still follows the native reset behavior; Smart Animate and Figma's `resetVideoPosition` preservation are not implemented.

AAC is decoded once per cached source on the background worker, with cancellation between access units. The player waits for initial frames and decoded audio before starting playback. One output stream mixes independent voices, using mono/stereo conversion and linear resampling to the output device's rate. Each voice's output-device frame counter drives its video clock; muting keeps that clock advancing silently. Repeated sources share immutable PCM rather than duplicating it. Adding or removing active voices rebuilds the single stream while retaining matching voice positions; this can briefly interrupt output when an overlay changes membership. Ordinary play, pause, mute, seek and loop operations do not rebuild the stream.

Audio device or stream errors are visible. Closing the player or replacing the source document stops the previous output; a preview pane suspends its audio while the full presenter is open. Stale decoder results and audio callback commits are rejected after identity or seek changes. No allocation, decoding or blocking mutex acquisition occurs in the audio mixer callback.

## Media interactions

The interaction editor supports Play, Pause, Toggle play/pause, Mute, Unmute, Toggle mute/unmute, Jump forward, Jump backward and Set to specific time. A media action targets all direct video fills on its destination node in the triggering presentation scope. An omitted destination means the triggering node. Times use finite, nonnegative seconds and clamp to the clip's duration. Runtime state changes do not create document-history entries.

Video timestamp and Video end triggers run through the existing ordered interaction and conditional-action path. The following edge behavior is an explicit FigCraft convention pending direct comparison with an editable Figma video file: timestamps fire when playback crosses them, not when a decoder finishes or an explicit seek changes position. Zero-time triggers fire when a traversal starts. Loops and restarts create new traversals. An explicit seek while paused does not fire an end event; Play at the end starts a new traversal from zero. Events are collected before dispatch, bounded per tick, and invalidated when their presentation scope is removed.

REST mapping supports `ON_MEDIA_HIT` with `mediaHitTime`, `ON_MEDIA_END`, and `UPDATE_MEDIA_RUNTIME` with the nine documented operations. Invalid times, unknown operations and unresolved explicit destinations produce import diagnostics. No undocumented autoplay/loop source fields are guessed. `.fig` export keeps its poster fallback and explicitly reports unsupported media actions and triggers; it does not turn a media trigger into a click trigger.

Primary references: [Figma video prototyping](https://help.figma.com/hc/en-us/articles/8878274530455-Use-videos-in-prototypes), [prototype actions](https://help.figma.com/hc/en-us/articles/360040035874-Prototype-actions), [Trigger API](https://developers.figma.com/docs/plugins/api/Trigger/), and [Action API](https://developers.figma.com/docs/plugins/api/Action/).

## Platforms and remaining scope

Linux desktop builds need the ALSA development library (`libasound2-dev`); installed packages need `libasound.so.2`. FreeBSD builds use `alsa-lib`. Flatpak manifests grant the audio-server socket. The browser has no CPAL dependency. The AppImage and tar.gz builds do not bundle it, so `libasound.so.2` becomes a hard runtime requirement of the binary on Linux; the deb and rpm packages declare it. Output devices with more than two channels get the left and right signal in their first two channels; mono devices get a downmix. Video edit lists that trim, delay or change speed are refused, while audio edit lists keep their bounded handling.

Placement and playback require the desktop app. The browser preserves existing video assets in native files and renders their posters; it does not bundle the new decoder. Its prototype view explains that moving video requires the desktop app.

Live sessions and online publishing of video documents are not supported in this slice. Hosting or publishing a document containing video is refused before contacting the transport. Adding video is rolled back while a host connection is pending, before its welcome, as well as after the live session is established. Use a local `.figcraft` file to transfer the complete source and poster. A live-session server also refuses any change that carries a video or a video paint, so a peer cannot add one. The same refusal covers a published snapshot and a saved session being restored.

A general media timeline, broader audio formats, navigation-state preservation and verified real `.fig` video import remain separate work.

## Validation status

Model, archive, audio-mixer, player and import regression tests cover the behaviour described above, including preservation of the source and poster through save and load, versions, branches, the clipboard and local libraries. Native desktop control checks with generated synthetic inputs covered Autoplay, Loop and Sound, all nine media actions, timestamp and end triggers, independent repeated-source instances, global pause that keeps individual pauses, saved-source deduplication, and reopen. Public synthetic fixtures cover audio shorter and longer than the video, pause, resume, restart and final-frame hold.

The native audio counters show samples submitted through the output callback with nonzero values; they do not prove acoustic output. The measured count of accepted frame results is not display FPS and is not a 30-FPS guarantee. Linux and browser runtime playback were not verified by hand, and the browser remains poster-only. No editable Figma video reference was available, so undocumented edge behavior is an explicit native convention and not a claim of visual parity.
