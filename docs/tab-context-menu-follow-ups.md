# Tab context menu follow-ups

The first tab-menu pass includes the small, self-contained actions: rename a file, close a tab,
close other tabs, close all tabs and open a tab in a new window. The remaining actions from the
reference menu need product decisions or broader UI/engine work; keep them out of the initial
implementation and revisit them when preparing a PR for that larger work.

| Follow-up | Why it needs more work |
|---|---|
| Pin tabs | Define whether pinned state is per window or per file, how it persists, and how pinned tabs reorder and close. |
| Copy Link | Define which links are available (local file, online share, or both), permission handling, and clipboard behavior when the file has not been shared. |
| Reload from disk | Detect external changes and define conflict/unsaved-edit handling before replacing the open document. |
| Tab groups | Design group creation, naming, colour, collapse/expand and persistence, then support dragging tabs into and between groups. |
| Move to another existing window | The app can open a tab in a new window; moving it between existing windows needs a window/tab registry and cross-window transfer rules. |
| Open in split tab | Requires a split-view layout and decisions for focus, resizing, closing, and per-pane document state. |
| Open in browser | Define a browser-compatible document URL and whether this means a local web preview or a shared online file. |

Before implementing a follow-up, check the ranked [gaps list](gaps.md) and the relevant
area-specific parity document. If it is already tracked there, update that entry rather than
creating a duplicate; otherwise add the work to the appropriate parity document and rank it in
`gaps.md` when its scope and priority are agreed. Update the affected status and revision history
when the work lands.
