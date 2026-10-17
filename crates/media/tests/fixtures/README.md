# Synthetic media fixtures

These files are original procedural test data generated for the FigCraft native video probe. FFmpeg `testsrc2` generated 12 frames at 96×64 and 6 fps; `libx264` encoded the silent MP4 with Baseline profile, 4:2:0, and a six-frame GOP. A separate fixture adds a synthetic 440 Hz AAC track to verify explicit audio rejection. The RGB reference was decoded from the silent file with FFmpeg to compare product output.

No Figma source, private media, incumbent asset, or product iconography was accessed or included. FFmpeg is a test-data/reference tool only; it is not a FigCraft runtime dependency.

The adapter validates SPS-derived decoder DPB capacity at no more than 16 frames; its independent RGBA output cache holds at most four frames.

Colorimetry tests use small planar ilmcraft_h264::Picture values built in the Rust test code; no additional media fixture or external source is used.
