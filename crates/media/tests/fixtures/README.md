# Synthetic media fixtures

These files are original procedural test data generated for FigCraft's native media adapter. FFmpeg `testsrc2` generated 12 frames at 96×64 and 6 fps; `libx264` encoded the silent MP4 with Baseline profile, 4:2:0, and a six-frame GOP. A separate small fixture adds a synthetic 440 Hz AAC track for audio decoding tests. `synthetic-h264-main-aac.mp4` uses `testsrc2=size=1920x1080:rate=30:duration=1` and `sine=frequency=440:sample_rate=48000:duration=1.25`; `libx264` encodes Main profile with CABAC, no B frames, one reference, and a 30-frame GOP, while AAC-LC encodes the mono tone. It exercises the bounded 1080p path and audio longer than video. `synthetic-h264-short-aac.mp4` uses 12 Baseline frames over two seconds with a one-second 660 Hz mono AAC-LC tone; it verifies that shorter audio does not shorten the video duration. The RGB24 reference for the small silent fixture and selected RGBA pixel values for the 1080p fixture were decoded independently with FFmpeg.

No Figma source, private media, incumbent asset, or product iconography was accessed or included. FFmpeg is a test-data/reference tool only; it is not a FigCraft runtime dependency.

The adapter validates SPS-derived decoder DPB capacity at no more than 16 frames; its independent RGBA output cache holds at most four frames. Audio output is bounded to 24 MiB of interleaved PCM and 3000 AAC access units.

Colorimetry tests use small planar `filmcraft_h264::Picture` values built in the Rust test code; no additional media fixture or external source is used.
