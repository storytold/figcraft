# Optional external file providers (web)

The web editor can delegate native project persistence to one **trusted same-origin host**.
The capability is **disabled by default**. Merely loading the editor exposes a protocol object;
it does not enable storage, render provider Recents, change ordinary local saves, or contact a server.
A host must explicitly call registerProvider. No server, account system, login, VM, domain,
filesystem mount or cloud vendor is built into this feature.

## Contract

window.figcraftFileProviders has protocolVersion 1 and app figcraft. Events are dispatched on window
as figcraft-file-provider CustomEvents; detail contains bounded metadata and, on save,
a Uint8Array named bytes. The host subscribes before registering, and can discover an already-ready
editor through the protocol object. The capability is independent of the optional control API.

| Method | Arguments / result |
| --- | --- |
| registerProvider | (providerId, label, namespace) -> connectionId. Repeating the exact identity is idempotent. |
| unregisterProvider | No arguments. Refused while an operation is unresolved; clears account resources and bindings. |
| setEnabled | (boolean). Suspends new provider choices/Recents without discarding pending work. |
| setResources | (connectionId, resources). Replace this account's native project list. |
| requestOpen | (resourceId) -> requestId. IDs must occur in the current connection's resource list. |
| openResource | (requestId, filename, Uint8Array). Replies only to a live, single-use open request. |
| cancelOpen | (requestId, error). Empty error means cancellation. |
| resolveSaveDialog | (requestId, destination, filename). destination is provider, device or cancel. |
| saveResult | (requestId, status, resourceId). status is saved, download, failed or cancel. |
| snapshot | Returns provider/namespace/connectionId, enabled, resources, documents and pending state. |

The namespace must identify the authenticated account, not its display name. A new connection ID
and per-page cryptographic request nonce prevent old account/page callbacks from matching new work.
This is correlation and separation of identities, **not authentication**. The host and its backend
must authenticate users, authorize every list/read/write, and scope durable storage by account.
Do not trust a filename, user-supplied account ID, browser page or request ID as an authorization check.
Only the hosting page's own script environment can use this capability; no postMessage listener,
foreign-origin message channel or filesystem authority is added.

A resource is an object with id, name and optional updatedAt strings. IDs are opaque and never used as
paths. The list is limited to 100 unique resources. Names must be native .figcraft filenames, at most
255 UTF-8 bytes, with no path separators or control characters. IDs/namespaces are at most 160 ASCII
characters (letters, numbers, dash, underscore, dot, colon). Labels are at most 100 UTF-8 bytes.
Native captures/read responses are limited to 80 MiB; retained transfer bytes and pending work are bounded.
The WASM adapter checks string/array lengths before copying host input and accepts only Uint8Array bytes.

## Save and Save As

Every interactive Save or Save As while enabled captures a native project and emits saveDialog.
The event carries requestId, documentId, revision, intent and suggested filename. The host presents
its own choice of **provider / device / cancel**, then calls resolveSaveDialog. The editor owns the
captured state while the choice is open, so changing tabs or editing cannot silently change the target.
No save happens merely because a dialog was opened or a byte capture was produced.

Choosing provider emits save with the immutable native bytes, filename, intent, documentId, revision,
providerId, namespace, connectionId and requestId. Save includes a previously bound resourceId;
Save As requests a distinct resource. The provider must finish its durable write before returning
saveResult(requestId, saved, resourceId). An ACK cannot nominate a document or revision, cannot be
replayed, and cannot bind a resource already owned by another open document. Failed/cancelled writes
keep edits unsaved. Later edits remain dirty when an older capture succeeds. There is no pretend local
path for an external resource.

Choosing device emits save with destination device and the capture. The host hands the bytes to the device using the
browser's ordinary download mechanism, then returns download as the result. This is a terminal transfer
result, **not durable provider confirmation**, and does not clear the document's saved checkpoint.
Save Copy, Export and programmatic commands keep their existing local behavior. With the capability
disabled or unregistered, the original browser/native save flow remains in use.

Provider Save/Save As always captures .figcraft, the editable native project with its embedded resources.
Use the existing local Copy/Export commands for interchange formats. The host must preserve the native
extension; changing a suffix cannot turn native bytes into PDF, SVG or another interchange format.
The native archive includes image assets, version history and branches. Capture tracks the main
document and open branch tabs; a later edit in any such tab is not acknowledged as saved.
Acknowledgement updates the captured document checkpoint by UID even when a non-content command,
such as opening another tab, increased its revision. Version/branch metadata is marked clean only
when the captured revision still matches; otherwise its unsaved flag is retained conservatively.
Existing live collaboration/server publication is separate from this storage capability.

Normal close/revert actions wait or refuse while provider work is unresolved. The browser can still
force-close/reload a tab; the host should provide an unsaved-work warning and must not acknowledge a
write it cannot recover. This bridge does not add automatic saving, cross-client conflict resolution,
offline persistence, autosave recovery or an inactivity/login implementation.

## Open and Recents

The existing Home screen and File > Open Recent display a separate provider section while enabled.
Local Recents retain their own storage and behavior. Publish only the current account's native files
with setResources; the editor never scans a VM or guesses a path from a name.

A click emits openRequested with requestId and resourceId. After authorization, the provider fetches
bytes and responds with openResource using that request ID. The editor parses the native project before
emitting opened or openFailed. Selecting an already-bound open resource focuses its existing document
without replacing unsaved edits. Unknown, duplicate or cancelled request replies are rejected.
The host handles failed fetches using cancelOpen rather than leaving the operation pending forever.

Completion events are saved, downloaded, saveFailed, saveCancelled, opened, openFailed and openCancelled.
Events associated with an account include providerId, namespace and connectionId. Storage keys should
use the provider/account/resource identities, never display names, and the backend should use versioning
or conditional writes if it allows concurrent clients.

## Validation

Regression fixtures cover the default-off gate, single-use/late replies, account reconnection, resource
scope and alias rejection, cancellation, immutable captures, switching tabs, edits after capture and
conservative device results. The source includes the WASM adapter and native encoder/decoder integration.
Run the ordinary repository checks and browser integration checks after publishing the complete suite's
source, as required by this contribution's development order. This document does not claim those checks
have been executed merely because fixtures are present.

Contributor credit: @alejocapo04.

## Native UI evidence

The existing recent-file integration tests include default-off and opt-in provider Home states.
The test checks the actual painted device/provider labels. With an output directory it also
rasterises those same native egui meshes and texture uploads, without a window or GPU:

```sh
CRAFT_PROVIDER_UI_FIXTURES=/absolute/output cargo test --locked -p figcraft-ui-egui --test it recent_file_names::provider_recents_render_separately_from_device_history -- --exact --test-threads=1
```

The outputs are `figcraft-provider-recents-off.png` and `figcraft-provider-recents-on.png`.
Names and resources are synthetic UI metadata, not an authentication service or content round-trip
fixture. The test-only CPU raster helper is adapted from SolidCraft's existing
[mesh renderer](https://github.com/tangency-labs/solidcraft/blob/57d789c0aaf1bcf4691647f9a1cb7a547eb45fdf/crates/ui-egui/tests/support/raster.rs),
with its MIT notice retained. No production dependency or rendering API is added.
