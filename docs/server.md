# figcraft-server

`figcraft-server` keeps FigCraft files for a team and serves their live sessions. A file published
to it stays there when everyone has left; people open it with a **share link** that lets them
**edit** it or only **view** it. Edits are kept as a snapshot plus a log of what changed, so a
restart (or a crash) loses nothing that was confirmed, and apps reconnect where they were. It is
headless, native only (no web build), and runs on the same platforms as the desktop app.

There are no user accounts yet: a share link is the access to one file, and the **server key**
manages files (publishing, links, removal). Accounts and per-person permissions come later. A file
can also hold a **team library** that other files use through one of its links (see
[Team libraries](#team-libraries)), and the server can keep a **plugin directory** for the team (see
[Plugin directory](#plugin-directory)). A server can also be **open to the public**: anyone
publishes without the key and manages their file with its **owner token**, within limits per
address (see [Running a public server](#running-a-public-server)).

The protocol and the design behind it are in [`collab-protocol.md`](collab-protocol.md).

## Running it

```sh
cargo build --release -p figcraft-server
target/release/figcraft-server --listen 0.0.0.0:47900 --data /srv/figcraft
```

Releases also ship it ready-built for Linux (`figcraft-server-<version>-linux-x86_64.tar.gz` and
`-linux-aarch64`). To run it for others on a rented server, see
[Deploy on a Linux server](#deploy-on-a-linux-server).

- `--listen` defaults to `127.0.0.1:47900`: only this machine can connect. `0.0.0.0:47900` (or an
  interface's address) lets the network in; put it behind a TLS reverse proxy before you expose
  it beyond a trusted network (see below).
- `--data` is the data directory (default `./figcraft-data`), made if missing. One server at a
  time uses a directory: a second one refuses to start.
- Logs go to standard output (`RUST_LOG=debug` for more). They name files, link ids and addresses,
  never keys, tokens or what files contain; what the operator watches for comes as `key=value`
  lines (`event=published file=… bytes=… ip=… by=public`, `event=refused ip=… reason=…`,
  `event=report`, `event=expired`, `event=blocked`).
- `GET /healthz` on the same port answers `200 ok` (on any path ending in `/healthz`), for uptime
  checks; nothing else listens.
- Stop it like any service (Ctrl+C, `systemctl stop`, `docker stop`, `kill`): on SIGTERM, SIGINT
  or SIGHUP (Ctrl+C or closing the console on Windows) it saves and compacts the open files and
  tells everyone it is restarting. Every change it accepted is on disk before anyone is told about
  it, so even a hard kill loses nothing confirmed (a file's log is then folded into a new snapshot
  the next time everyone leaves it). Apps that were in a file keep editing and reconnect once the
  server is back (for about a minute and a half), as the same participants.

## Publishing a file

```sh
figcraft-server --data /srv/figcraft key
```

prints the address to publish to, `ws://<host>:47900/?key=<server key>`. In the desktop app,
**File ▸ Publish to server…** (or **Share ▸ Advanced ▸ Publish to another server…**) asks for that address and the
file's name. The open file is uploaded as a new file and the tab keeps syncing through the
server; until the upload is done the tab is read-only ("Publishing…" in the toolbar). The Share
popover then shows an **edit link** and a **view link** to copy. Agents do the same with
`collab.publish {server, name?}` ([`mcp.md`](mcp.md#live-sessions)).

Whoever has the server key can publish files and manage every file's links: give it only to
people you trust.

On a server [open to the public](#running-a-public-server) the address alone is enough
(`wss://figcraft.example.com/`): the dialog asks the server first and shows that no key is needed,
who runs it ("Hosted by …" and a link to its terms) and how long unopened files are kept. Publishing
then also gives an **owner token**, shown once with the links in a **File published** window. The
app keeps it with your preferences and lists the file under [Home ▸ Online](#your-online-files),
where it can be opened, shared, renamed and removed later without the token in hand; copy it if
you want to manage the file from another computer or a script (`collab.manageFile` from
[MCP](mcp.md#live-sessions)). The Share popover shows the server's line
and the date the server deletes the file unless someone opens it.

## Sharing with a join code

**Share ▸ Online ▸ Start sharing** in the app (`collab.share`) is the short way to do all of the
above on a server open to the public: it publishes the open file to the server in **Preferences ▸
Server for online sharing** (the FigCraft server unless you put yours there; Reset goes back; empty
turns online sharing off), then shows a **join code** such as `K7M-2QX-9TD` in place of the links.
Someone else types it under **Share ▸ Join with a code** (or on the home screen, or
`collab.join {code}`) with the same server in their Preferences, and is in the file. The popover
switches between the code that lets people edit and the one that only lets them view.

The file is its owner's: the app keeps the file's first links and its owner token for itself and
makes two more links ("Shared (edit)", "Shared (view)" in `figcraft-server links list`) for
everyone else, with their codes. **Stop sharing** revokes those two: their codes and links stop
working at once, whoever joined with them is disconnected ("Sharing was stopped…"; they keep the
copy they have), and the owner stays in the file, which stays on the server, open only to them.
**Start sharing** again gives the same file new links and codes. **Share ▸ Advanced ▸ Remove from
server…** (it asks first) takes the file off the server; closing the tab or quitting changes
nothing on the server, and the file is still yours afterwards: see
[Your online files](#your-online-files). To use **your** server for this, set its address
(`wss://figcraft.example.com/`) in Preferences on every machine that shares or joins; a server
that needs its key is reached under **Advanced ▸ Publish to another server…**, or with
`collab.share {server: "wss://…/?key=…"}`, and gets codes too.

- **A code is a name for one share link**: nine symbols, 45 random bits, from an alphabet without
  look-alike characters (typing ignores case, spaces and dashes). The server hands one out when
  whoever manages the file asks (the app does, as the file's owner, or with the server key), one
  per link, so the view code and the edit code differ. Typing a code gives the joiner's app the
  link's token and it joins with that: access, reconnects and revocation are the link's.
- **Nothing readable is kept**: `links.json` gets the code's stretched hash and the link's token
  wrapped with the code, never the code. A code goes when its link is revoked, its file is removed
  or expires, or a newer code is asked for the same link.
- **Guessing is limited on every server**, public or private: 10 codes a minute per address; 10
  wrong in a row lock an address out of codes for 15 minutes ("Too many wrong codes from your
  address…"); and 1,000 wrong an hour from everyone together pause codes for all ("Join codes are
  paused on this server…") until the budget refills. Links are never affected. Refused codes are
  logged as `event=code_refused ip=… reason=…`, used ones as `event=code_used file=… ip=…`, new
  ones as `event=code file=… link=…`; the codes themselves are never logged.
- **Upgrade note.** Join codes need a server of this version or newer, and so does everyone's app.
  An older server still takes files and links from a newer app (the app shows the links where the
  code would be, and says a typed code can't be asked there); a newer server serves older apps
  unchanged. To hand out codes, upgrade the server binary and restart it: the data directory needs
  no migration, existing files get codes the next time they are shared, and no setting turns codes
  on or off. Behind the reverse proxy nothing changes (codes travel inside the WebSocket).

## Your online files

A file you put on a server without its key is yours, and stays where it is when its tab closes or
the app quits. The app remembers each one **with your preferences on this computer** (never in the
document): the server, the file's id and name, its owner token, your own links to it, when you put
it there and last opened it, and the code and links others join with while it is shared. The home
screen lists them under **Online**:

- **A row** shows the file's name, the server, **Shared** or **Not shared**, and when you last
  opened it. Clicking it opens the file as its owner, in step with the server, in a tab of its
  own (or brings its tab to the front when it is open already). In that tab **Stop sharing** and
  **Start sharing** work as in the session that first shared it.
- **Its menu** (the button at the row's end): **Open**; **Stop sharing** / **Start sharing**
  (others lose their links and codes, or get new ones; the file itself stays); **Copy code** and
  **Copy link** while it is shared; **Rename on the server…**; **Remove from server…** (it asks
  first: the file is deleted there for everyone and can't be brought back); **Forget on this
  computer** (the row goes, **the file stays on the server** until it expires, and without its
  token this computer can't manage it again).
- **Nothing is asked of a server until you open Online** (or act on a file). Then each server is
  asked once, in the background, what became of your files there. A file the server no longer has
  (removed, or expired unopened) reads **No longer on the server** and can be cleared from the
  list. A server that can't be reached says so above the list, with **Try again**; the rows still
  show what this computer remembers.
- **Tabs** of files in a live session carry a small mark after the name: waves for a session
  others get into (a file you share or joined), a closed ring for a file of yours on a server that
  only you get into now. Messages about a file that isn't in front name it ("Ada joined
  Homepage").
- **After a crash**, a file of yours that was open opens again from the server as its owner, and
  the recovery copy of it is kept until it is known that nothing in it is missing there: changes
  of yours that never reached the server (it was unreachable, or the app died first) are put back
  on top and sent to everyone ("Recovered your changes to …"); if others changed the file in the
  meantime the copy opens beside it as "… (recovered copy)" for you to compare; if the server
  can't be reached the copy opens as an unsaved document, as before.

The owner token is a password for that one file: it is kept only in the preferences file of your
user account (`state.json` in FigCraft's settings folder), is never logged, and never appears in
what commands return (`collab.files` says `hasOwnerToken: true`). Whoever can read that file can
manage your online files; a list of at most 200 files is kept (the one opened longest ago makes
room). The files are the ones published **from this computer**: another computer of yours doesn't
know them unless you manage them there with the token. A file published with the server's key has
no owner token and belongs to the server's operator: it isn't listed.

Scripts and agents use `collab.files {refresh?}`, `collab.openFile {file}`,
`collab.forgetFile {file}` and `collab.manageFile {file, action}` ([MCP](mcp.md#live-sessions)).

- **Upgrade note.** Asking what became of the files needs a server of this version or newer. An
  older server still works for everything else: the list shows what the computer remembers, says
  that the server can't tell what became of the files, and opening, sharing, renaming and removing
  go on as before (they use requests the server already knows). To get the state of the files,
  upgrade the server binary and restart it: the data directory needs no migration and there is no
  setting to turn on.

## Joining

A [join code](#sharing-with-a-join-code) is the short way in. Share links look like the desktop
app's invite links and join the same way (the same **Join with a code** field takes them, as do
**File ▸ Join live session…**, opening the link, and `collab.join {link}`):

- `figcraft://join?host=<host:port>&file=<id>&token=<token>` for a plain `ws://` server;
- `figcraft://join?server=wss://figcraft.example.com/&file=<id>&token=<token>` for a server behind
  a TLS reverse proxy or under a path;
- `ws://<host:port>/?file=<id>&token=<token>` or `wss://…` (the same link as a WebSocket URL).

The file opens as a new tab. With a **view link** you see everyone's changes and presence, and
others see your cursor and selection, but you can't change anything: the toolbar shows a **View
only** badge with only the Move and Hand tools, edits are refused with a message (and the server
refuses them too). **Leave session** keeps the file open as your own copy, editable again.

## Team libraries

A file on the server can hold a **team library**: its published components, styles and variables,
which other files use without anyone copying the library file around.

- **Publishing.** In the library file, opened from the server with an edit link (or published to
  it), **Assets ▸ Libraries ▸ Publish to server** publishes the chosen assets as the library's next
  version (`library.publishToServer`; with the server's address and key a file not on the server
  yet is kept there first). The server keeps the latest version of each file's library. Each asset
  keeps the version it last changed in, so files using the library update only what changed; the
  server's version is the one that counts, so when someone published since, the next publish
  follows it. Publishing needs an edit link: a view link's publish is refused.
- **Using.** In another file, paste one of the library file's links (a **view link** is enough)
  into **Assets ▸ Libraries** (`library.add {path: <link>}`). The library is listed in the Assets
  panel like a library file: placing a component or applying a style copies it in as a read-only
  linked copy, as for library files. While the Assets panel shows, the app asks the server for a
  newer version at once and then every 30 seconds (a few bytes when nothing changed) and shows
  **Library updates available**; accepting keeps instance overrides. While the server can't be
  reached, the last version fetched stays in use. The file keeps the link, token included:
  sharing that file shares read access to the library.
- **Usage.** Each time a file's app asks for the library it reports which of its components the
  file uses and how often. **Show usage** in the library file (`library.usage`) lists the files and
  the instances of each component. Files are told apart by their names; a file's report changes
  only when its usage does.
- **Swapping.** `library.swap` (the swap button by an enabled library) moves a file's instances,
  styles and variables from one library to another's of the same names, keeping overrides.

The app never waits for the server: publishing, adding a library by its link and **Show usage**
send their requests on a thread of their own, show that they're on their way (with **Cancel**) and
finish when the answer arrives, acting on the file they started in. Scripts, the CLI and the MCP
server run the same commands and wait for the answer.

The browser build can't use team libraries yet (it has no library support of its own).

## Plugin directory

The server keeps a directory of plugins ([`plugins.md`](plugins.md#plugin-directories)) on the same
port as everything else; no other service is involved.

- **Who can do what.** Anyone who can reach the server lists, searches and downloads its plugins
  (no link or key needed: put the server behind your network or proxy if that's too open). Only
  whoever has the **server key** publishes and unpublishes, the same key that publishes files: there
  are no publisher accounts, so give it only to people you trust to put code on your team's
  machines. A plugin's author is shown as the publisher typed it in the manifest; nothing verifies
  it.
- **Publishing.** `figcraft-cli plugin publish FOLDER --server ws://host:47900/` with the key in
  `FIGCRAFT_SERVER_KEY` (or `?key=` in the address), or `plugin.directory.publish` from the app's
  scripts and MCP. Each version must be newer than the last (`1.0.0` → `1.0.1`); the server keeps
  every version until it is unpublished, and lists the latest.
- **What the server checks** before keeping a bundle: its size (`max_plugin_mb`), that it decodes
  (an uncompressed format: nothing expands), that its manifest parses and asks for a plugin API this
  server's FigCraft runs, a version of one to four numbers newer than the one kept, that the module
  is WebAssembly that compiles and imports only what FigCraft's sandbox provides (it is never run on
  the server), and that asset names are plain file names. Then the caps: `max_plugins` versions in
  all and `max_plugins_mb` of bundles.
- **Downloads** carry the bundle's SHA-256, which the app checks before reading it, and which the
  server checks again when it reads the bundle back from disk (a damaged one is never sent, and is
  logged). Downloads share a budget of about 32 MiB a second, so many at once are turned away with
  "busy" rather than queued in memory.
- **Moderation** is the operator's: `figcraft-server plugins` lists every plugin and its versions,
  `figcraft-server plugins remove ID [--version V]` takes a version, or the whole plugin, down (it
  works on a running server too). Installed copies keep working; users can't tell the server to
  remove anything without the key.
- **Turning it off**: `max_plugins = 0`.

## Managing files and links

| Command | What it does |
|---|---|
| `figcraft-server key [--rotate]` | The address to publish to, with the key; `--rotate` makes a new key (the server must be stopped) |
| `figcraft-server check` | Check the settings (after editing the `--config` file) |
| `figcraft-server files [list] [--sort size\|age\|idle]` | The files: id, name, people in it now, size, created, days unused, expiry, reports, and who published it (`key`, or the address of a file published without it); biggest, oldest or longest unused first |
| `figcraft-server files remove FILE` | Remove a file and its links for good (whoever is in it is disconnected) |
| `figcraft-server links list FILE` | A file's links: id, access, created, label |
| `figcraft-server links create FILE --access edit\|view [--label TEXT]` | A new link, printed once |
| `figcraft-server links revoke FILE LINK` | Revoke a link: it no longer joins, and whoever joined with it is disconnected |
| `figcraft-server plugins [list]` | The plugin directory: every plugin, its versions, author, access, size and SHA-256 |
| `figcraft-server plugins remove ID [--version V]` | Unpublish a version of a plugin, or every version |
| `figcraft-server reports` | What people in files reported to you: when, which file, the link and address they came with, why |
| `figcraft-server block-ip IP` | Refuse an address (an IPv6 address: its /64) and drop its connections; kept in `blocked.json` |
| `figcraft-server unblock-ip IP` / `blocked` | Accept it again / list the blocked addresses |

Each takes `--data` (or `--config`). When a server is running on the directory, the command asks
it over the same port with the server key (so a revoked link's users are disconnected at once);
otherwise it changes the directory itself. A new link is shown only when it is made: the server
keeps only a hash of its token, so a lost link can't be recovered (revoke it and make another).
Publishing makes an edit link and a view link labelled `Published (edit)` and `Published (view)`.

## Configuration

Every flag is also a key of a TOML file given with `--config server.toml` (flags win; unknown keys
are errors):

| Key | Default | Meaning |
|---|---|---|
| `listen` | `127.0.0.1:47900` | Where to listen |
| `data` | `figcraft-data` | The data directory |
| `public_url` | (the listen address) | How links name the server, when people reach it another way (a reverse proxy: `wss://figcraft.example.com/`) |
| `max_files` | 1000 | Most files kept |
| `max_file_mb` | 64 (16 when open) | Largest file (its snapshot), in MiB; a welcome carries the whole file, so messages may be as large as a file and 1 MiB (at least 64 MiB on a private server). The app opens files of up to 1023 MiB; above 64 with `open_publish` the server warns |
| `max_links` | 100 | Most share links per file |
| `max_connections` | 512 | Most open connections |
| `max_people` | 256 | Most people in one file at once |
| `compact_after` | 1000 | Compact a file's log into a new snapshot after this many entries… |
| `compact_mb` | 16 | …or once its log is this large (MiB) |
| `max_log_mb` | 64 | Most MiB of recent batches a file in use keeps in memory (and in its snapshot) for apps that reconnect; an app further behind catches up with the whole file. Open to the public, at most a quarter of `max_file_mb` too |
| `max_plugins` | 500 | Most plugin versions the plugin directory keeps (all plugins together); 0 turns the directory off |
| `max_plugin_mb` | 16 | Largest plugin bundle, in MiB (at most 32) |
| `max_plugins_mb` | 1024 | Most MiB of plugin bundles kept |
| `open_publish` | false | Anyone may publish without the key ([a public server](#running-a-public-server)) |
| `operator_contact` | (none) | How to reach you about abuse (an email address or a page), shown to people where they publish |
| `terms_url` | (none) | Your terms of use (`https://…`), linked where people publish |
| `trusted_proxy` | (none) | The reverse proxy's address (`127.0.0.1` for Caddy or nginx on the same machine): only its `X-Forwarded-For` is believed |
| `idle_days` | 30 | Files published without the key are deleted after this many days nobody opened them (0: kept; fractions allowed) |
| `max_total_mb` | 10240 | Publishing without the key stops while all files take this many MiB |
| `max_files_per_ip` | 20 | Open: most files one address published without the key… |
| `max_mb_per_ip` | 256 | …and their MiB |
| `max_connections_per_ip` | 32 | Open: most connections from one address at once |
| `connects_per_minute` | 30 | Open: new connections an address may open a minute (a burst of as many) |
| `publishes_per_hour` | 10 | Open: files an address may publish without the key an hour |
| `messages_per_second` | 200 | Open: messages an address may send a second (a burst of four seconds' worth) |

With `listen` on every interface (`0.0.0.0`) and no `public_url`, links name this machine's
network address. The limits marked *open* apply only with `open_publish = true`; a private server
behaves as before (blocked addresses, report rates and the limits on
[join codes](#sharing-with-a-join-code) apply to every server).

## The data directory

```text
<data>/LOCK                    held while the server (or a command) uses the directory
<data>/server.json             where the running server listens (read by the commands)
<data>/server.key              the server key
<data>/files/<id>/meta.json    the file's name, when it was created and last changed
<data>/files/<id>/links.json   its links: id, access, label, created, SHA-256 of the token, and for
                               a link with a join code its stretched hash and the wrapped token
<data>/files/<id>/snapshot.bin its state at the last compaction (with a checksum)
<data>/files/<id>/log.bin      what changed since, one checksummed record per change
<data>/files/<id>/library-<version>.bin  its team library, when one was published (a .figcraft
                               archive; library.bin when it was published by an older server)
<data>/files/<id>/usage.json   which files use that library's components, as their apps report it
<data>/plugins/index.json      the plugin directory: every version kept (manifest fields, size, SHA-256)
<data>/plugins/<sha256>.bin    a plugin bundle, named by the SHA-256 of its bytes
<data>/blocked.json            the addresses the operator blocked
<data>/reports.json            the last 1,000 reports people sent about files
```

A file published without the key also has, in its `meta.json`, the SHA-256 of its owner token,
the address it came from and when it was last opened (for expiry).

- **Writes are atomic**: a snapshot, a description or a link list is written to a temporary file
  beside it, flushed, then renamed over the old one. The log is only ever appended to, and synced
  before the server tells anyone about the change.
- **Compaction** writes the file's state as the next snapshot and starts an empty log, after
  `compact_after` entries or `compact_mb` of log, when the last person leaves a file that was
  edited (the ids of people who only joined stay in the log), and when the server stops. A log left over from a compaction cut short is replayed over the snapshot, which already
  holds it all, so nothing changes. If a damaged generation field made a current log look like such
  a leftover, its edits the snapshot lacks are recovered the same way and the damage is reported.
- **Damage is skipped and reported, never deleted.** At startup a folder whose description, links
  or snapshot can't be read is left out and logged; a log that ends in a cut-short or damaged
  record loses only that end (logged when the file opens). Leftover temporary files are removed.
- The server key is stored as plain text (on Unix readable by the owner only): whoever can read
  the data directory can read every file anyway. Link tokens are stored only as hashes.
- An index entry whose bundle is missing or the wrong size is left out at startup and reported;
  bundles are deleted only when their last version is unpublished. A `plugins/index.json` that
  can't be read lists nothing and refuses publishing (nothing is written over it) until it is
  restored or removed.

## Backups

[`packaging/server/figcraft-backup.timer`](../packaging/server/figcraft-backup.timer) (installed by
`install.sh`; `sudo systemctl enable --now figcraft-backup.timer`) does the following every night:
it checks there is twice the data directory's size free, stops the server for the seconds the copy
takes, keeps `/var/backups/figcraft-server/figcraft-YYYY-MM-DD.tar.gz` (readable by root only) for
7 days and starts it again. Copy that folder off the machine too. The archives hold every file,
the addresses files came from, the reports and the **server key**: keep them, and any copy, as
private as the server itself.

Copy the data directory (the plugin directory, `plugins/`, included) while the server is stopped,
or take an atomic snapshot of the filesystem it lives on (ZFS, Btrfs, LVM, a VSS shadow copy). A plain copy of a running server's
directory can catch a file between its snapshot and its log; it restores, but may lose that
file's changes since its last compaction. To restore, stop the server, put the directory back and
start it: apps that were in a file catch up with a fresh copy (their unsent edits are kept when
the server still knows them, else they are told what couldn't be kept).

## Behind a reverse proxy (TLS)

The server speaks plain WebSocket (`ws://`); TLS comes from a reverse proxy in front of it, which
gives people `wss://` links (and browsers the `wss://` they need on an `https://` page).

- Run the server on the loopback interface (`listen = "127.0.0.1:47900"`), and set
  `public_url` to the address people use, for example `wss://figcraft.example.com/`. Links and
  the publish address then name the proxy.
- **Caddy** ([`packaging/server/Caddyfile`](../packaging/server/Caddyfile)): a site block for
  your host name with a `reverse_proxy` to `127.0.0.1:47900`. Caddy obtains the certificate and
  passes WebSocket upgrades through on its own.
- **nginx** ([`packaging/server/nginx.conf`](../packaging/server/nginx.conf)): a `server` block
  on 443 with your certificate that passes requests to `http://127.0.0.1:47900` over HTTP/1.1
  with the `Upgrade` and `Connection` headers forwarded (that is what lets the WebSocket upgrade
  through), and a read timeout above the server's 45-second idle limit (apps ping every 15
  seconds).
- **Under a path**: the server answers on any path, so the proxy can forward a prefix such as
  `/figcraft/` unchanged; set `public_url = "wss://example.com/figcraft/"`.
- Tokens never travel in the WebSocket request (the client sends them inside its first message),
  so the proxy's access logs don't record them.

The desktop app, `figcraft-cli` (and so agents through MCP) and the browser build all open
`wss://` links. The desktop app and the CLI check the certificate against Mozilla's list of root
authorities built into them (not the operating system's store), so a certificate from a public
authority such as Let's Encrypt just works. For a certificate from a private authority (or a
self-signed one, such as Caddy's `tls internal`), set the environment variable `FIGCRAFT_CA_FILE`
to that authority's certificate (a PEM file) before starting the app: it is trusted in addition
to the built-in list. There is no way to skip the check: a certificate that has expired, is for
another name or comes from an unknown authority is refused, and the app says which.

## Running a public server

A server **open to the public** (`open_publish = true`) lets anyone publish a file without the
server key and share its links: hosting for people you don't know, with no accounts. The operator
keeps the key for everything else.

- **Publishing.** The app asks the server about itself first (`collab.serverInfo`): it is open, who
  runs it (`operator_contact`), its terms (`terms_url`) and how long unopened files are kept. A file
  published without the key gets its edit and view links and an **owner token** (128 random bits,
  shown once, kept only as its SHA-256): with it, whoever published the file lists, makes and
  revokes its links, renames it on the server or removes it (`collab.manageFile`), asks whether
  it is still there (what the app's [Online list](#your-online-files) does, for all its files in
  one request), and nothing else.
  An app older than open hosting is told the server is open when it tries a key.
- **Limits per address** (only when open): connections at once (`max_connections_per_ip`), new
  connections a minute (`connects_per_minute`), files published an hour (`publishes_per_hour`),
  messages a second (`messages_per_second`), and files and MiB kept (`max_files_per_ip`,
  `max_mb_per_ip`, counting what files grow to by edits); for everyone: `max_files`, `max_file_mb`
  per file (also as it grows), `max_total_mb` and 32 addresses' worth of publishing an hour. An
  address is an IPv4 address or an IPv6 /64; an IPv6 /56 and /48 also count together, with 2 and 4
  times one address's limits, and a /48 never more than an eighth of any budget everyone shares
  (connections, queued bytes, bytes being read, publishing, downloads). Each refusal says why ("too many connections from your address…",
  "this server is full…") and the app shows it. See also [Resource limits](#resource-limits).
  Behind a reverse proxy set `trusted_proxy` to its address: the server then counts the client its
  `X-Forwarded-For` names (the last address in it, the one the proxy added) and believes the header
  from no one else. The tables of addresses keep at most 10,000 addresses (the least recently seen
  go first), so floods of addresses cost bounded memory.
- **Expiry.** A file published without the key is deleted once nobody has opened it for
  `idle_days` (30 by default; opening it, editing it and leaving it all count as use). The server
  looks every ten minutes and at startup, never deletes a file someone is in, and tells everyone in
  a file the date it would go (the Share popover shows it). Files published with the key never
  expire.
- **Team libraries** on a public server live only in files published with the key: a file published
  without it can't hold one (a library is a whole archive the server unpacks and checks).
- **Moderation.** `figcraft-server files list --sort size|age|idle` shows who published what
  (the address) and how many reports each file has; `files remove FILE` takes a file down (whoever
  is in it is disconnected); `block-ip` refuses an address and drops its connections (kept across
  restarts in `blocked.json`; the operator's own key still gets in from anywhere); anyone in a file
  can report it (**Report this file…** in the Share popover, `collab.report`; ten an hour per
  address), and `figcraft-server reports` lists the reports with the link and address they came
  from. Plugin directory publishing stays key-only.
- **Your responsibility.** As the operator you answer for what is published on your server: give a
  working abuse contact (`operator_contact`) and terms (`terms_url`), read the reports, and remove
  what breaks your terms or the law where you run it. Files are **not end-to-end encrypted**: TLS
  protects them on the way, but whoever runs the server (and anyone who gets into the machine) can
  read every file, link and report, and the server keeps the address each file was published from.
  Say so in your terms.

At startup a public server warns in its log when `operator_contact` is empty, when it seems to be
behind a reverse proxy (a `wss://` `public_url`, or listening on the loopback interface) without
`trusted_proxy` (every client would then count as the proxy's address, so one person's limits would
stop everyone), and when `idle_days = 0`. `figcraft-server check` reads the settings without
starting.

### Threat model

What someone can do, by what they hold (the server is reached through TLS at the proxy):

| Who | Can | Cannot |
|---|---|---|
| Anyone who reaches the server, with nothing | Ask about its hosting (open, operator, terms, limits); on an open server publish files within the limits per address and for everyone; list and download the plugin directory; ask `/healthz`; try join codes, within the limits on wrong ones | Join, read or list any file; see links, reports or blocked addresses; use or learn the server key or anyone's token (none are ever sent back, and all are compared in constant time against hashes); publish plugins; spoof their address through `X-Forwarded-For` (believed only from `trusted_proxy`); get past a block |
| Someone with a **join code** | Exactly what its link gives (a view code: the view link's row; an edit code: the edit link's), until the code or its link is gone | Get a code for any other link; learn anything from a wrong code but that it is wrong |
| Someone with a file's **view link** | See that file and everyone's changes and presence in it; read its team library and usage; report it | Change it (the server refuses a viewer's edits); reach any other file; manage links |
| Someone with a file's **edit link** | Change that file, within its size cap; publish its team library (on a public server only for files published with the key); report it | Reach any other file; grow it past `max_file_mb` or its publisher's share; list, make or revoke links; rename or remove it |
| Whoever holds a file's **owner token** (published without the key) | List, make and revoke that file's links (disconnecting whoever joined with a revoked one, which is how the app stops sharing), ask for join codes for links whose token they hold, rename it on the server, remove it; ask whether it is still there, its name, link ids and expiry (asking doesn't count as opening it) | Touch any other file, even with its id; tell a file that doesn't exist from one they don't own; list files; read reports; block; anything of the plugin directory |
| The **operator** (server key, or the machine) | Everything above for every file, reports, blocks, the plugin directory; read every file on disk | Recover a lost link or owner token (only hashes are kept) |

Limits that hold against everyone: message sizes by what a connection is for, file sizes as they
grow, connections in all and per address and block, connection, message, publish and download rates
per address and block, files and MiB per address and for everyone, the bytes queued for slow
clients and waiting for the server, and idle expiry ([Resource limits](#resource-limits)). Revoking
a link or removing a file cuts every connection that used it, library connections included. Not
defended against: someone with many unrelated addresses (each gets its own limits; the global caps
still hold, so they can fill the server up to them until you block them or the files expire), a
link or owner token that leaks (revoke links with the owner token or `figcraft-server links
revoke`; remove the file), and the operator reading files (no end-to-end encryption).

### Resource limits

What keeps one client (or a few) from exhausting a small machine (the official one has 2 GB of
memory and 2 GB of swap; the defaults are chosen to hold on 1 GB). The rule throughout: no single
address or block of addresses can use up a budget everyone shares (each gets at most an eighth),
and when a shared budget runs out the biggest consumer is dropped, not the next client that happens
to need a little of it. These apply only with `open_publish = true` unless marked *always*.

**On private servers too** (a change from earlier versions): a batch that would take a file past
`max_file_mb` (64 MiB by default there) is now refused with the reason, and the app rolls it back,
where it used to be kept and only logged; and nothing is written while the disk has less than
1 GiB (or two of the largest files) free. Raise `max_file_mb` (at most 4096) if your team's files
are larger.

| Resource | Limit |
|---|---|
| Message size | A file's size and 1 MiB (`max_file_mb`, 16 by default when open, or `max_plugin_mb` if larger): checked as the message is read, before anything is decoded; 64 KiB on a connection that only browses plugins (a client sending more is disconnected with the limit as the reason; the app keeps its own copy). On a private server, the larger of that and the core's 64 MiB |
| A file as it grows | Every batch is checked first (*always*): past `max_file_mb`, past its publisher's `max_mb_per_ip` or past `max_total_mb`, it is refused with the reason and the app rolls it back. A long log is compacted to make room at most once a minute per file (a compaction rewrites the whole file). A file's snapshot also holds the recent batches kept for reconnecting clients, at most `max_log_mb` and a quarter of `max_file_mb` |
| Recent batches in memory | A quarter of `max_file_mb` (256 KiB at least) per open file; older ones are dropped (a client that missed them catches up with the whole file) |
| Bytes queued for slow clients | The largest message and 1 MiB per connection, 32 MiB per block of addresses, 256 MiB for everyone. A message sent to many people counts once for everyone and once per block (twenty colleagues behind one office address cost their block one copy of a broadcast), and welcomes (bounded by the file's size) don't count against blocks, so a whole office can join a file at once. A client that doesn't read its answers is dropped; when everyone's budget runs out, the connection holding the most is dropped |
| Bytes being read | Messages not yet whole: the largest message and 1 MiB per block, 192 MiB for everyone; past everyone's budget the readers holding the most are cut off until it fits (their bytes count as free at once), and a reader is refused only when it is the biggest |
| Bytes waiting for the server | 128 MiB of read messages not handled yet: past it a connection is dropped as busy |
| Connections | `max_connections` (512) in all (*always*); `max_connections_per_ip` (32) per address, ×2 per IPv6 /56 (at most a sixteenth of all), ×4 per /48 (at most an eighth of all), joins included, so no block can take every connection; one-shot requests (asking about hosting, libraries, browsing plugins) can't take the last eighth, which stays for people joining files, and are dropped after 20 idle seconds |
| Rates | Per address and block: `connects_per_minute`, `publishes_per_hour` (and 32 addresses' worth for everyone), `messages_per_second` (cursors, selections, cursor chat and pings have their own budget beside it, twice that, so an office moving its cursors together doesn't trip it); reports, ten an hour (*always*); downloads of libraries and plugins, 1 MiB a second with bursts of 16 MiB per address (*always*), checked before anything is read, within 32 MiB a second (512 MiB at once) for libraries and for plugins, everyone together |
| Team libraries | Only files published with the key hold one on a public server; an archive is at most `max_file_mb` and everything it unpacks to counts against the same cap (*always*), so a zip bomb is refused before it expands |
| Usage reports | Kept in memory, written at most once a minute per library and when the server stops; at most 1,000 components per file and 200 files per library (*always*) |
| Disk writes | A batch is confirmed only once it is on disk; when its log can't be written, nobody hears it was accepted: the file closes, everyone in it reconnects and their apps send again what wasn't confirmed. Below 1 GiB (or two of the largest files) free, changes and new files are refused with "the server's disk is nearly full" instead (*always*, where the free space can be told: Unix). The server keeps the last 200 problems for the operator |
| Storage | `max_files`, `max_total_mb` (10240), `max_files_per_ip` (20), `max_mb_per_ip` (256), and idle expiry (`idle_days`, 30) for files published without the key. Size `max_total_mb` to the disk: the files, a compaction's copy of the largest, the nightly backups (each about the data's size, seven kept unless you move them off) and 1 GiB to spare |
| The service | systemd caps it at 60% (slows down) and 75% (stopped and restarted) of the machine's memory, and only lets it talk to this machine (the proxy) |

The settings for a public server, as `install.sh` writes them:

```toml
listen = "127.0.0.1:47900"
data = "/var/lib/figcraft-server"
public_url = "wss://figcraft.example.com/"
trusted_proxy = "127.0.0.1"
open_publish = true
operator_contact = "abuse@example.com"
terms_url = "https://example.com/figcraft-terms"
idle_days = 30
max_file_mb = 8
max_plugin_mb = 8
max_total_mb = 6144
max_files_per_ip = 20
max_mb_per_ip = 256
messages_per_second = 600
connects_per_minute = 60
```

These are the recommended values for a small public server (2 GB of memory, a 25-50 GB disk with
the nightly backups on it): files up to 8 MiB keep a whole office's welcomes within the queues,
6 GiB of files leaves room for a week of backups, and the per-address rates leave room for an
office behind one address.

### On a fresh Ubuntu server with install.sh

[`packaging/server/install.sh`](../packaging/server/install.sh) sets up everything below on a new
Debian 12+ or Ubuntu 24.04+ machine (a cloud provider's smallest plan, 1 GB of memory, is enough to
run it). It installs the binary of the release archive it comes in, or builds the copy of the
source it sits in, or else downloads the latest release for the machine (checked against
`SHA256SUMS.txt`) or builds a fresh clone (`--from-source`); a build adds a temporary 2 GB swap
file on a machine with less than 2 GB of memory, so it isn't killed. It makes the `figcraft` user, the
data directory and `/etc/figcraft-server/server.toml` (only when missing: your edits are kept),
installs the systemd units and its Caddy site (`/etc/caddy/figcraft-server.caddy`, imported from
the main Caddyfile, whose other sites it leaves alone), opens the SSH ports `sshd` listens on, 80
and 443 in `ufw`, and turns `ufw` on unless other services listen on the machine (then it says
so: allow their ports and enable it yourself, or pass `--firewall-anyway`). Builds run as the
unprivileged `figcraft-build` user on a copy of the source (root installs nothing from that copy;
the swap file and the built binary, copied out with the build user's rights, live in the root-only
`/var/cache/figcraft-install`), and the binary is only ever run as `figcraft` (never as root);
a fresh clone builds the latest release tag. A Caddyfile with other sites is never replaced: it
gains one `import figcraft-server.caddy` line, once (`packaging/server/install-test.sh` checks
this). It checks new settings before they replace anything, starts it all and prints what to do next. Running it again
upgrades the binary and restarts the server only when something changed.

1. Create the server (Ubuntu 24.04, x86_64 or arm64) with your SSH key, and note its address.
2. At your DNS provider, point `figcraft.example.com` (A, and AAAA for IPv6) at that address. Caddy
   needs it to get the certificate.
3. Log in and run the script, from a release archive:

   ```sh
   ssh root@figcraft.example.com
   VERSION=$(curl -fsSL https://api.github.com/repos/storytold/figcraft/releases/latest | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p')
   curl -fLO "https://github.com/storytold/figcraft/releases/download/v$VERSION/figcraft-server-$VERSION-linux-$(uname -m).tar.gz"
   tar -xzf figcraft-server-*.tar.gz && cd figcraft-server-*/
   sudo ./install.sh --host figcraft.example.com --email abuse@example.com --terms https://example.com/figcraft-terms
   ```

   or from a copy of the source (a private repository, or unreleased changes): it then builds that
   copy as it is (with the swap file on a small machine).

   ```sh
   git archive --format=tar.gz -o figcraft-src.tar.gz HEAD          # on your machine
   scp figcraft-src.tar.gz root@figcraft.example.com:
   ssh root@figcraft.example.com 'mkdir -p figcraft && tar -xzf figcraft-src.tar.gz -C figcraft &&
     figcraft/packaging/server/install.sh --host figcraft.example.com --email abuse@example.com'
   ```

   Without `--host` or `--email` it asks for them. `--private` keeps publishing key-only;
   `--backups` turns the nightly backup on; `--no-firewall` leaves `ufw` alone. To upgrade, copy
   the newer source over and run it again.
4. Check it: `curl https://figcraft.example.com/healthz` answers `ok` once Caddy has its
   certificate (a minute after DNS is right); `journalctl -u figcraft-server -f` shows the log.
5. In FigCraft, **Share ▸ Publish to server…** with `wss://figcraft.example.com/`. Moderate with
   the commands it printed (`sudo -u figcraft figcraft-server --config /etc/figcraft-server/server.toml
   files list --sort size`, `reports`, `block-ip`).
6. Point an uptime monitor at `https://figcraft.example.com/healthz`, copy
   `/var/backups/figcraft-server` off the machine, and upgrade by running the script again.

## Deploy on a Linux server

Hosting for people outside your network: a small Linux server (1 CPU and 1 GB of memory are
plenty for a team), a DNS name pointing at it (`figcraft.example.com` below: use yours), the
server behind Caddy or nginx for TLS, and systemd keeping it running. The commands are for Debian
and Ubuntu; other distributions differ only in how packages are installed. `install.sh` does all of
it ([above](#on-a-fresh-ubuntu-server-with-installsh)); these are the steps by hand.

**1. Install the binary**, from the latest release (x86_64 or aarch64), checked against the
release's `SHA256SUMS.txt`:

```sh
VERSION=$(curl -fsSL https://api.github.com/repos/storytold/figcraft/releases/latest | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p')
NAME="figcraft-server-$VERSION-linux-$(uname -m)"
curl -fLO "https://github.com/storytold/figcraft/releases/download/v$VERSION/$NAME.tar.gz"
curl -fLO "https://github.com/storytold/figcraft/releases/download/v$VERSION/SHA256SUMS.txt"
sha256sum --ignore-missing -c SHA256SUMS.txt
tar -xzf "$NAME.tar.gz" && cd "$NAME"
sudo install -m 755 figcraft-server /usr/local/bin/
```

or from source (it needs a C compiler for the TLS library; the example files are then in
`packaging/server/`):

```sh
sudo apt install -y build-essential git curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && . "$HOME/.cargo/env"
git clone https://github.com/storytold/figcraft && cd figcraft
cargo build --release --locked -p figcraft-server
sudo install -m 755 target/release/figcraft-server /usr/local/bin/
cd packaging/server
```

**2. A user of its own, its data directory and its settings:**

```sh
sudo useradd --system --home-dir /var/lib/figcraft-server --shell /usr/sbin/nologin figcraft
sudo install -d -o figcraft -g figcraft -m 700 /var/lib/figcraft-server
sudo install -d -m 755 /etc/figcraft-server
sudo tee /etc/figcraft-server/server.toml >/dev/null <<'EOF'
listen = "127.0.0.1:47900"
data = "/var/lib/figcraft-server"
public_url = "wss://figcraft.example.com/"
trusted_proxy = "127.0.0.1"
EOF
```

(Add the settings of [a public server](#running-a-public-server) to open it to everyone.)

**3. Run it with systemd.** [`figcraft-server.service`](../packaging/server/figcraft-server.service)
runs it as that user, restarts it if it fails, stops it gracefully, and locks it down: no new
privileges, a read-only system with only the data directory writable, no capabilities, private
`/tmp` and devices, a system-call filter, memory caps (60% of the machine slows it down, 75%
stops and restarts it, so only this service is hit) and network only to this machine (the proxy;
remove `IPAddressAllow`/`IPAddressDeny` to listen on a network address without one).

```sh
sudo cp figcraft-server.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now figcraft-server
systemctl status figcraft-server        # logs: journalctl -u figcraft-server
```

**4. TLS in front of it.** Point the DNS name's A (and AAAA) record at the server first, then set
`HOST` to it. Either **Caddy** (simplest: it gets and renews the certificate itself; packaged in
Debian 12+ and Ubuntu 24.04+):

```sh
HOST=figcraft.example.com
sudo apt install -y caddy
sed "s/figcraft.example.com/$HOST/" Caddyfile | sudo tee /etc/caddy/Caddyfile >/dev/null
sudo systemctl reload caddy
```

or **nginx** with a certificate from Let's Encrypt (certbot renews it):

```sh
HOST=figcraft.example.com
sudo apt install -y nginx certbot python3-certbot-nginx
sudo certbot certonly --nginx -d "$HOST"
sed "s/figcraft.example.com/$HOST/g" nginx.conf | sudo tee /etc/nginx/conf.d/figcraft.conf >/dev/null
sudo nginx -t && sudo systemctl reload nginx
```

**5. Firewall: only SSH, 80 and 443.** Port 80 answers certificate challenges and redirects to
https; the server's own port stays closed (it listens on the loopback interface anyway).

```sh
sudo ufw allow OpenSSH
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw enable
```

**6. Publish and share.** Print the publish address (`wss://figcraft.example.com/?key=…`) and use
it in the app's **File ▸ Publish to server…** (see [Publishing a file](#publishing-a-file)):

```sh
sudo -u figcraft figcraft-server --config /etc/figcraft-server/server.toml key
```

The other commands ([Managing files and links](#managing-files-and-links)) run the same way, as
the `figcraft` user with `--config /etc/figcraft-server/server.toml`.

**In Docker instead of systemd.** [`packaging/server/Dockerfile`](../packaging/server/Dockerfile)
builds a small image (the binary on a distroless base, no shell) that runs as an unprivileged
user, keeps its files in the `/data` volume and stops gracefully on `docker stop`. Build it from
the repository root, give it a settings file with your `public_url`, and publish its port on the
loopback interface only (ports Docker publishes bypass `ufw`), with the proxy from step 4 in front
(connections then come from Docker's bridge, so a `trusted_proxy` is that bridge's gateway address,
`172.17.0.1` by default):

```sh
docker build -f packaging/server/Dockerfile -t figcraft-server .
sudo install -d /etc/figcraft-server
printf 'listen = "0.0.0.0:47900"\ndata = "/data"\npublic_url = "wss://figcraft.example.com/"\n' | sudo tee /etc/figcraft-server/server.toml
docker run -d --name figcraft-server --restart unless-stopped -p 127.0.0.1:47900:47900 \
  -v figcraft-data:/data -v /etc/figcraft-server/server.toml:/etc/figcraft-server/server.toml:ro figcraft-server
docker exec figcraft-server /usr/local/bin/figcraft-server --config /etc/figcraft-server/server.toml key
```

**Upgrading.** Install the new binary as in step 1, then restart: the restart is graceful and apps
reconnect on their own. An app speaking another protocol version is refused with a message saying
so: upgrade apps and server together when the release notes say the protocol changed.

```sh
sudo systemctl restart figcraft-server
```

With Docker: rebuild the image, then `docker rm -f figcraft-server` and the `docker run` above
again (the volume keeps the files).

**Backups.** See [Backups](#backups); with systemd, for example:

```sh
sudo systemctl stop figcraft-server
sudo tar -C /var/lib -czf "figcraft-backup-$(date +%F).tar.gz" figcraft-server
sudo systemctl start figcraft-server
```

## Security

- **Not end-to-end encrypted.** TLS (from the reverse proxy) protects files on their way to and from
  the server; on the server they are stored unencrypted, so the operator and anyone who can read
  the data directory can read every file. Tell the people who publish to your server so (the
  README's [Privacy and online sharing](../README.md#privacy-and-online-sharing) says it for the
  default public server, which a build can replace or leave out with `FIGCRAFT_DEFAULT_SERVER`
  ([releasing](releasing.md#the-default-server)), and whose terms are [online-sharing-terms.md](online-sharing-terms.md): a
  starting point for your own), and set `operator_contact` and `terms_url` on a public one.
- **Access** is a share link (128-bit random token) per file and access level, an owner token
  (128-bit) per file published without the key, or the server key (128-bit) for managing files.
  Tokens are stored as SHA-256 hashes and every comparison runs in constant time; revoking a link
  disconnects whoever joined with it. A view link's batches are refused by the server whatever the
  client does.
- **Join codes** are 45 random bits naming one link: far easier to guess than a link, so every
  server limits wrong codes per address and for everyone
  ([Sharing with a join code](#sharing-with-a-join-code)); with those limits, finding any one of
  20,000 live codes takes on the order of centuries, while a flood of wrong codes can pause codes
  (never links) for a while. Codes are kept only as a stretched hash and compared in constant
  time; the link's token is kept wrapped with the code. A copy of the data directory lets someone
  try codes offline (10,000 hashes a guess), and it holds the files themselves anyway.
- **Open to the public** (`open_publish`): anyone who reaches the server may ask about it, publish
  a file (within the limits above) and report a file they are in; an owner token manages only its
  own file (its links, its name, removing it). Listing files, other files' links, reports, blocking
  and the plugin directory's publishing stay with the server key, which also gets in from blocked
  addresses. The per-address limits trust `X-Forwarded-For` only from `trusted_proxy`; without
  one, every connection counts as its own address.
- **Transport**: plain `ws://` on the server's port; anyone on the network path can read traffic
  (presence, changes, tokens) unless TLS terminates in front of it. Use a reverse proxy for
  anything beyond a trusted network ([Deploy on a Linux server](#deploy-on-a-linux-server)).
- **Limits**: the transport's (message sizes, 100 messages a second per connection, idle and
  handshake timeouts, `max_connections`) and the collaboration core's (layers, changes per batch,
  `max_people`: the cap per file), plus `max_files`, `max_file_mb` and `max_links`, and on a public
  server the limits per address ([Resource limits](#resource-limits)). Every message is decoded
  defensively, and fuzzed in the tests (including open hosting, owner tokens and reports). A
  library is read in full before it is kept (everything it unpacks to within `max_file_mb`), must
  be the file's library at a newer version, and is checked on the server's main thread; usage
  reports keep at most 1,000 components per file and 200 files per library.
- **Team libraries**: any link of a file reads its library and its usage reports (file names and
  counts); only an edit link publishes it.
- **Plugin directory**: anyone who reaches the server reads it; only the server key writes to it.
  Everything published is checked in full and never run on the server; a bundle's name on disk is
  its hash, so nothing a publisher chooses reaches the file system; downloads are hash-checked at
  both ends and share a bandwidth budget. Installing still means trusting the publisher: a plugin
  runs in FigCraft's sandbox with the permissions its manifest asks for, which the app shows before
  it installs anything.
- **Not end-to-end encrypted**: the server reads every file it keeps (it validates and stores them);
  so can its operator.
- **The machine**: the data directory is made readable by the server's account only and the server
  key is written owner-only from the start; the systemd unit caps memory and network
  (`MemoryHigh`, `MemoryMax`, `IPAddressAllow=localhost`); `install.sh` builds as an unprivileged
  user, runs the binary only as the server's account, never replaces someone else's Caddyfile and
  never cuts off SSH or other services when it turns `ufw` on.
- **Not there yet**: user accounts, per-person permissions, audit logs, end-to-end encryption.
