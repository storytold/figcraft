# Server memory follow-up

Joining a persisted session issues a client id and makes its journal dirty, even if nobody edits
the document. Previously, the last client leaving caused a full snapshot rewrite for that id.
For image-heavy documents this creates large temporary allocations and unnecessary disk writes;
an allocator can retain the freed memory after the room closes.

On idle, the server now flushes the journal and only compacts when the document sequence is newer
than the stored snapshot, or when flushing failed and a snapshot must be attempted as a fallback.
Issued ids and refusals remain durable in the journal. Existing size/count compaction thresholds
remain in effect, and edited documents still compact when the last participant leaves.

Snapshot encoding also appends the versioned postcard body directly to the snapshot header's
buffer, then fills the checksum. It no longer holds a separate complete encoded body alongside
the finished snapshot. The bytes, generation, checksum and file format are unchanged.

Regression coverage uses synthetic documents: six read-only sessions across store reopenings
must preserve increasing client ids without rewriting the snapshot; exact legacy framing and
checksum must match; the binary message body must be unchanged when appended to a prefix.
Existing crash recovery, damaged-input and edit compaction tests continue to apply.

This reduces unnecessary temporary allocations. It does not promise that the operating system
immediately reclaims every freed allocation, or that all large-document memory costs are solved.
