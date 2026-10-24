# Contributors in the About window

**About ▸ Contributors** credits everyone who contributed to FigCraft, and **About ▸ Models** credits
the AI models named in `Co-Authored-By` trailers. This follows the shared craftrules standard
[`standards/contributors.md`](https://github.com/storytold/craftrules/blob/main/standards/contributors.md);
this page is the local copy of the decision.

## Decision

- Three files are **compiled into the binary** by the UI crate's `build.rs` (the web build has them too):
  - [`contributors/contributors.json`](../contributors/contributors.json): commit stats per GitHub
    username. **Generated** by `craftrules/scripts/contributors.py`; never hand-edit.
  - [`contributors/people.toml`](../contributors/people.toml): the names people chose for
    themselves. **Each contributor adds only their own entry**, in their own PR.
  - [`contributors/models.json`](../contributors/models.json): a small list of AI models credited
    in commit trailers, updated separately from the generated contributor statistics.
- When Contributors opens, FigCraft asks GitHub's public API for the current commit contributors
  and merged PR authors. The request is asynchronous and uses no GitHub account or token. It runs
  once per app session unless the viewer presses Refresh. The live view shows usernames, commit
  counts and merged PR counts; it includes people whose only contribution was a merged PR. The API
  may report commits a few hours late. If GitHub cannot be reached, the built-in snapshot is shown
  with a clear notice and a Retry button. Nothing else in About depends on the request.
- The built-in snapshot also carries display names (opt-in, or their synced GitHub profile name),
  real names (only if they told us), lines added/deleted, binary assets and commit dates. Live
  usernames use those names only when a verified snapshot entry for that username exists.
- Models uses the built-in `models.json` list. It lists AI
  tools named in `Co-Authored-By` trailers, including Claude and Codex. Opening the tab does not
  scan commit history over the network; the list changes when `models.json` is updated for a new
  build. `source_head` records the main commit used for the current model list.
- The list is shown as a **grab bag** (names flowing as text) or a **table**, in the same order. The
  live list sorts by name, merged PRs or commits; the built-in snapshot also sorts by line counts,
  binary assets and commit dates. The name toggle cycles **Username → Display name → Real name**;
  a missing name falls back to `@username`. Alphabetical sorting is case-insensitive and ignores
  the `@`.

## Credit yourself

Add your entry to `contributors/people.toml` in a PR you commit yourself (every field optional):

```toml
[people.your-github-username]
real_name = "Your Name"
display_name = "Nickname"
sync_github_name = true   # use the name on your public GitHub profile as display name
```

or, with craftrules checked out next to `craft-apps/`:

```sh
python3 ../../craftrules/scripts/contributors.py --add-me . --real-name "Your Name" --sync-github-name
```

Names show after the next refresh: the script checks that each entry was committed by its own
user (or by a maintainer with a `consent_source`). Delete your entry to remove your names.

## Refreshing (maintainers)

```sh
python3 ../../craftrules/scripts/contributors.py .           # regenerate + verify; needs git, Python 3.11+, gh
python3 ../../craftrules/scripts/contributors.py --check .   # verify people.toml only
```

Commit the updated `contributors/contributors.json`. Git author names and emails are never recorded.
