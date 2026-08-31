# 🪢 lasso

**Review what your agents actually did — and talk back.**

A review pane for [herdr](https://herdr.dev): live diffs of everything an
agent changed, across every repo it touched, with inline comments that go
straight back to the agent as a single prompt.

[![CI](https://github.com/skellleks/lasso/actions/workflows/ci.yml/badge.svg)](https://github.com/skellleks/lasso/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/skellleks/lasso)](https://github.com/skellleks/lasso/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

```
┌Agent───────────────────┐┌diff: bot/handlers/orders.py─────────────────────────┐
│● digital_goods  idle   ││    446      user = await service.get_by_id(user_id) │
└────────────────────────┘│    447      if user is None:                         │
┌Files───────────────────┐│    448          raise HTTPException(404)             │
│  ▾ digital_goods_bot   ││    449 -    configs = await svc.get_all(user_id)     │
│    ▾ bot               ││    449 +    configs = await svc.get_all_serialized(… │
│      ▾ handlers        ││●   450      return {"configs": configs}              │
│M       orders.py       ││       └ why not paginate this?▏                      │
│M     config.example.py ││    451                                               │
└────────────────────────┘└──────────────────────────────────────────────────────┘
 j/k h/l nav · ⇥ focus · ↵ view · c comment · S submit · b base · a all · q quit
```

## Why

When several agents work in parallel, "what did this one actually do?" is
the question you ask all day. lasso opens next to an agent's pane and stays
**pinned to that agent forever** — its diff, its files, its review. Comments
can never accidentally go to the wrong agent; open one lasso window per
agent instead.

## Features

- **Whole-file diffs** — full file as context, not bare hunks; opens at the
  first change. Syntax highlighting, green/red fills, word-level emphasis of
  the exact tokens that changed (delta-style), deleted lines shown inline.
- **Multi-repo** — repos the agent edited outside its own cwd (discovered
  from its Claude Code session transcripts) appear in the tree as top-level
  nodes with their own diffs.
- **Review comments** — `c` on any line (changed or not, in the diff or the
  file viewer), drafts persist across restarts, `Shift+S` sends everything
  to the agent as one prompt with paths, line numbers and quoted context.
- **Live** — refreshes when the agent's status changes (socket subscription)
  and every few seconds; your reading position survives refreshes.
- **File tree** — changed files by default, whole repo with `a`; collapsible
  dirs, `M`/`A`/`D` badges; untracked files shown as fully added.
- **Mouse everywhere** — click to select agents/files/lines, wheel to scroll
  any pane, horizontal wheel (or `Shift`+wheel) for long lines.
- **Diff base toggle** — `b` switches HEAD ⇄ merge-base with main/master.

## Install

```sh
herdr plugin install skellleks/lasso
```

The build step downloads a prebuilt binary for your platform from GitHub
Releases (sha256-verified) and falls back to `cargo build` if none matches.
Prebuilt targets: macOS (Intel & Apple Silicon), Linux (x86_64 & arm64 musl).

Open a window next to the focused agent:

```sh
herdr plugin pane open --plugin lasso --entrypoint review
```

Or bind a key in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+d"
type = "plugin_action"
command = "lasso.open"
description = "open lasso review"
```

then `herdr server reload-config`. The action is smart about windows: it
opens next to the agent you invoked it from, replaces that agent's existing
window instead of duplicating it, and acts as a close-toggle when pressed
from inside a lasso pane.

## Keys

| Key | Action |
| --- | --- |
| `j`/`k`, arrows | move cursor / navigate |
| `h`/`l`, ←/→ | horizontal scroll of long lines |
| `Tab` | switch focus: files ⇄ diff |
| `Enter` | file: open in viewer · dir: fold/unfold |
| `d` / `Esc` | back to the diff |
| `c` | comment on the current line |
| `x` | delete the last comment |
| `Shift+S` | submit all comments to the agent as one prompt |
| `b` | diff base: HEAD ⇄ merge-base |
| `a` | files: changed ⇄ whole repo |
| `r` | refresh |
| `q` | quit |

Submitting to a busy (`working`/`blocked`) agent asks for confirmation
first. Comment drafts are stored per agent in the plugin state dir, so
closing the window keeps your unfinished review.

## How it works

lasso talks to herdr over its socket API: `agent list` for the pinned
agent's cwd and status, `events.subscribe` for live refresh, and
`agent prompt` to deliver the review. Diffs come from plain `git` in the
agent's repos. Cross-repo changes are found by reading the agent's Claude
Code transcripts (`~/.claude/projects/…`) for Edit/Write tool calls and
grouping the touched files by git root — repos with no pending changes are
skipped. Comments on files outside the agent's cwd carry absolute paths so
the agent can act on them from wherever it runs.

## Development

```sh
cargo test          # 115 tests: diff parser, tree, review store, app state, rendering
cargo build --release
herdr plugin link .                 # register this checkout in a running herdr
herdr plugin pane open --plugin lasso --entrypoint review
```

Standalone mode (outside herdr) shows the diff of the current directory —
handy for hacking on lasso itself; comments and submit are disabled there.

Releases: bump `version` in `herdr-plugin.toml` and `Cargo.toml`, update
`CHANGELOG.md`, then `git tag v<version> && git push --tags` — GitHub
Actions builds and uploads the binaries the install script expects.

## License

[MIT](LICENSE)
