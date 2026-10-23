# Display-sized network images

The native handshake optionally echoes `X-FigCraft-Previews: figcraft.previews.v1`, only together
with the binary-assets extension. Existing protocol enum variants keep their wire indexes;
`FetchPreview`, `FetchPreviews` and `Preview` are appended. Older clients/servers retain the original-image path.

A preview request names an original content reference and a power-of-two maximum side (64–1024).
The hub requires a joined, negotiated connection and checks that the original belongs to that
file before serving even a cached derivative. A global two-worker pool generates sRGB PNGs with
alpha outside the collaboration hub thread. Work is deduplicated, capped at 32 distinct pending
jobs, 32 recipients per job and 64 MiB of queued encoded sources (one oversized source alone).
Decoding is bounded to 128 MiB per job. A busy/unsupported preview falls back to the original path.

The guest has at most eight preview requests in flight, independent of the two original downloads.
It coalesces them into batches of at most eight over 5 ms and rate-limits preview messages to
50/s (100-message burst) so a warm gallery does not trigger the host's 100/s message limiter.
Replies must match an outstanding request; PNG signature, encoded size (8 MiB) and dimensions are
validated. Client cache identities include the locally derived authenticated file scope. A
64 MiB/2048-entry memory cache on each side holds disposable previews; a restart regenerates them.
They are never document images, undo entries or replacements for originals. Arrivals invalidate
retained canvas pixels through the existing asset-generation path.

Image paints choose a pixel level from the original image-to-canvas mapping and display scale.
Decoded and padded caches distinguish levels; a close view cannot reuse a small level as its
original. The decoded cache's entry ceiling is 4096, within its existing 512 MiB pixel budget, so
hundreds of small cards can coexist. Original aspect, crop and rotation use the original dimensions.
Saves/exports still obtain the originals and refuse missing resources; source bytes are unchanged.

The real-socket regression `small_view_downloads_a_preview_and_zoom_and_save_keep_the_original`
checks a 1600×2400 synthetic image: only the small PNG arrives for an overview; the original is
absent from the disk cache until explicitly required, then a native save contains identical bytes.
