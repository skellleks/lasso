# lasso

Review pane for [herdr](https://herdr.dev): see what each agent changed,
leave inline comments on the diff, and send them back to the agent as one
prompt. Plus a syntax-highlighted file viewer.

## Layout

```
┌─Agents──────┬── diff: user.py ─────────┐
│● api      3 │ @@ -41,6 +41,9 @@        │
│○ web      2 │  def get_user(...):      │
├─Files───────┤-    return None          │
│M user.py    │+    user = repo.find(id) │
│M config.py  │+    return user          │
└─────────────┴──────────────────────────┘
```

- **Agents** — every agent pane herdr knows about, with status badge;
  the diff on the right is the uncommitted changes in that agent's cwd.
- **Files** — changed files of the selected agent (`a` toggles all files).
- Untracked files are shown as fully added. Diff auto-refreshes when an
  agent's status changes (socket subscription) and every 5s.
- **Multi-repo**: when the agent edited files outside its own repo (per its
  Claude Code session transcript), those repos appear in the tree as
  top-level nodes with their own diffs. Comments then carry absolute paths
  so the agent can act on them from any cwd.
- Each window is pinned to one agent forever; open one window per agent
  (re-invoking the hotkey on the same agent replaces its window, on the
  lasso pane itself closes it).

## Keys

Full mouse support: click agents/files/diff lines to select, wheel to
scroll any pane, horizontal wheel (or Shift+wheel) to scroll long lines.

`a` switches the Files pane to a full file tree (dirs collapsible by
click/Enter); changed files carry M/A/D badges, and opening a file shows
its changes inline — added lines marked in the gutter, deleted lines in
red where they used to be.

| Key | Action |
| --- | --- |
| `j`/`k`, arrows | navigate |
| `h`/`l`, ←/→ | horizontal scroll of long lines |
| `Tab` | switch focus: agents → files → diff |
| `Enter` (on file) | open file viewer |
| `d` / `Esc` | back to diff |
| `c` | comment on the current diff line |
| `x` | delete last comment |
| `S` | submit all comments to the agent as one prompt |
| `b` | diff base: HEAD ⇄ merge-base with main/master |
| `a` | files: changed ⇄ all |
| `r` | refresh |
| `q` | quit |

Comments are persisted in the plugin state dir, so closing the pane keeps
your draft review. Submitting to a non-idle agent asks for confirmation.

## Install

```sh
herdr plugin link /path/to/lasso     # local dev
herdr plugin pane open --plugin lasso --entrypoint review
```

Keybinding (herdr config.toml):

```toml
[[keys.command]]
key = "prefix+d"
type = "plugin_action"
command = "lasso.open"
description = "open lasso review"
```

Standalone (outside herdr) `lasso` shows the diff of the current
directory; comments/submit are disabled.

## Dev

```sh
cargo test
cargo build --release
```
