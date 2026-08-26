# Changelog

## [Unreleased]

## [0.1.0] - 2026-08-26

### Added

- Review pane pinned to one herdr agent: whole-file diffs with syntax
  highlighting, add/del fills and word-level change emphasis.
- File tree of changed files (and of the whole repo with `a`), collapsible
  dirs, change badges.
- Multi-repo: repos the agent edited (discovered from its Claude Code
  transcripts) show up alongside the cwd repo.
- Inline comments on any line — in the diff and in the file viewer — sent
  to the agent as one prompt with `Shift+S`.
- Full mouse support, horizontal scrolling, adaptive status bar.
- One window per agent; the `lasso.open` action replaces that agent's
  window and toggles when invoked from the lasso pane itself.
