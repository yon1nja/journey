# Journey

**Context persistence for engineering efforts.**

[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Crates.io](https://img.shields.io/crates/v/journey.svg)](https://crates.io/crates/journey)
[![Build Status](https://github.com/yon1nja/journey/actions/workflows/ci.yml/badge.svg)](https://github.com/yon1nja/journey/actions)

Journey gives each engineering effort a durable folder on disk with metadata, a short description, linked git worktrees, effort-local docs, and lifecycle status. It is intentionally not a task manager, checkpoint system, agent orchestrator, or replacement for git — it preserves context around the work while leaving notes, plans, decisions, and version control to the tools you already use.

<p align="center">
  <img src="demo.gif" alt="Journey interactive TUI demo" width="800">
</p>

> [!NOTE]
> Journey is under active development and may contain bugs, rough edges, or behavior that changes between releases. Issue reports and feedback are very welcome.

---

## Table of Contents

- [Philosophy](#philosophy)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Core Concepts](#core-concepts)
- [Using Journey](#using-journey)
  - [Interactive Interface](#interactive-interface)
  - [Context Resolution](#context-resolution)
- [Journey Lifecycle](#journey-lifecycle)
- [Commands](#commands)
- [Interactive Actions](#interactive-actions)
- [Configuration](#configuration)
- [Upcoming Features](#upcoming-features)
- [Contributing](#contributing)
- [License](#license)

---

## Philosophy

Journey provides the smallest durable container for an engineering effort while staying deliberately unopinionated about the workflow inside it.

Each Journey answers **what** the effort is, **why** it exists, **where** its worktrees and notes live, and what **lifecycle state** it is in. Beyond that, Journey stays extensible and quiet — it does not decide how you plan, document, delegate, summarize, or reason about the work.

## Installation

### Homebrew

```sh
brew install yon1nja/tap/journey
```

Or:

```sh
brew tap yon1nja/tap
brew install journey
```

### From source

```sh
cargo install --path .
```

### Development

```sh
cargo build
cargo test
cargo run -- --help
```

Persistent state defaults to `~/.journey`. Set `JOURNEY_HOME` to use a different directory.

## Quick Start

```sh
# Create a Journey
journey new "Investigate auth failures" --description "Reproduce and fix intermittent login failures"

# Link the current repo
journey link .

# Capture a note
journey capture "Token refresh fails after idle timeout"

# Check status
journey status

# Pause and resume
journey pause
journey resume
```

Or just run `journey` to open the interactive TUI.

## Core Concepts

A **Journey** represents one engineering effort — an investigation, migration, review, or feature. It can span multiple repositories and multiple live git worktrees.

Each Journey stores:

| File / Directory | Purpose |
|---|---|
| `journey.yaml` | ID, title, description, status, creation time, linked repos |
| `journal.jsonl` | Append-only operational log (link, unlink, status changes) |
| `AGENTS.md` | Pointer to shared Journey agent guidance |
| `CLAUDE.md` | Claude-specific pointer to `AGENTS.md` |
| `README.md` | Optional top-level overview |
| `docs/` | User-owned Markdown files |
| `worktrees/` | Symlinks to attached git worktrees |

Global state under `JOURNEY_HOME`:

| File | Purpose |
|---|---|
| `JOURNEY-AGENTS.md` | Shared coding-agent guidance for all Journeys |
| `index.yaml` | Journey registry for list and TUI views |
| `worktree-index.yaml` | Worktree-to-Journey ownership mapping |
| `config.toml` | Action ordering and keyboard shortcuts |

## Using Journey

### Interactive Interface

Running `journey` with no arguments opens the interactive TUI. This is the most complete way to use Journey — it combines browsing, searching, details, document viewing, creation, and actions in one place.

**Default keybindings:**

| Key | Action |
|---|---|
| `a` | Enter search mode |
| `Esc` | Back / quit |
| `Ctrl-N` | Create a new Journey |
| `Enter` | Open details or run action |
| `Tab` / `Shift-Tab` | Switch document tabs |
| `Up` / `Down` / `PgUp` / `PgDn` | Navigate or scroll |

The TUI includes:

- **Search** — filter by title, ID, description, status, and linked repo names
- **Details pane** — metadata, lifecycle state, linked repos, recent events, and paths
- **Document tabs** — read `README.md` and `docs/*.md` inline with rendered Markdown
- **Actions pane** — configurable actions with keyboard shortcuts

### Context Resolution

Commands that operate on an existing Journey resolve context in this order:

1. Explicit ID from a positional argument or `--journey <id>`
2. Walking up from the current directory until `journey.yaml` is found
3. Matching the current directory against `worktree-index.yaml`
4. Error if no Journey can be found

This means `journey status`, `journey capture`, and doc commands work from inside a Journey folder or an attached worktree without repeating the ID.

## Journey Lifecycle

| Status | Description | Owns worktrees? |
|---|---|---|
| `active` | Current or ongoing work | Yes |
| `paused` | Not currently active, but resumable | Yes |
| `archived` | Completed or closed, kept for reference | No (released) |
| `abandoned` | Stopped, not expected to continue | No (released) |

A canonical worktree path can belong to only one active or paused Journey at a time.

When a Journey is archived or abandoned, its worktrees are detached from the global ownership index but **not deleted from disk**. The Journey folder and repo references remain intact.

When an archived or abandoned Journey is resumed, Journey attempts to reattach its worktrees. Reattachment fails if a worktree is missing or already owned by another active/paused Journey.

Lifecycle commands never snapshot, stash, restore, or compare code — git remains responsible for code state. The one exception is the interactive `done` action, which archives the Journey and removes linked worktrees after confirmation.

## Commands

### `journey` (no subcommand)

Opens the interactive TUI when stdout is a terminal. In non-interactive contexts, prints the active Journey list.

### `journey new`

```sh
journey new <title> [--description <text>]
```

Creates a Journey with `journey.yaml`, `journal.jsonl`, `AGENTS.md`, and `CLAUDE.md`.

### `journey list`

```sh
journey list [--status active|paused|archived|abandoned] [--non-interactive]
```

Lists Journeys. With `--non-interactive`, prints tab-separated rows: `<id> <status> <updated> <repos>`.

### `journey status`

```sh
journey status [<id>]
```

Prints a one-screen summary: title, description, ID, status, path, repo count, and event count.

### `journey link` / `journey unlink`

```sh
journey link <repo-path> [--name <name>] [--journey <id>]
journey unlink <repo-name> [--journey <id>]
```

Link or detach a git repo/worktree. Linking records the canonical path, current branch, and repo name, and creates a symlink under `worktrees/`.

### Lifecycle commands

```sh
journey resume [<id>]
journey pause [<id>]
journey archive [<id>]
journey abandon [<id>]
```

### `journey capture`

```sh
journey capture [--journey <id>] [--doc <name>] <text>
```

Appends a timestamped Markdown entry to `docs/capture.md`. Reads from stdin if no text arguments are passed.

```sh
journey capture "Recheck auth retry after config rollout"
printf "line one\nline two\n" | journey capture --doc investigation
```

### `journey doc`

```sh
journey doc new <name> [--journey <id>]
journey doc list [--journey <id>]
journey doc path <name> [--journey <id>]
```

Create, list, or print paths for Journey-local Markdown docs under `docs/`.

### `journey readme`

```sh
journey readme new [--journey <id>]
journey readme path [--journey <id>]
```

Create or print the path to the Journey's `README.md`.

### `journey doctor`

```sh
journey doctor [--repair]
```

Checks `worktree-index.yaml` for consistency. With `--repair`, rebuilds the worktree index from active and paused Journeys.

## Interactive Actions

| Action | Shortcut | Description |
|---|---|---|
| `open_claude` | `c` | Run `claude` in the Journey folder |
| `open_nvim` | `n` | Run `nvim` in the Journey folder |
| `new_branch_worktree` | `b` | Create a new branch + worktree, then link |
| `existing_branch_worktree` | `w` | Select existing branch, create/reuse worktree, then link |
| `link_current` | `l` | Link the current directory's git worktree |
| `unlink_repo` | `u` | Detach a linked repo (does not delete) |
| `capture` | `t` | Append a timestamped note |
| `delete_worktree` | `d` | Remove a linked worktree and unlink it |
| `done` | `f` | Archive + remove linked worktrees (with confirmation) |
| `pause` | `p` | Mark paused |
| `archive` | `x` | Mark archived, release worktree ownership |
| `copy_cd` | — | Copy `cd <path>` to clipboard |
| `resume` | — | Mark active, reattach worktrees |
| `abandon` | — | Mark abandoned, release worktree ownership |

## Configuration

Journey creates `config.toml` in `JOURNEY_HOME` on first use.

<details>
<summary>Default config.toml</summary>

```toml
[actions]
order = [
  "open_claude",
  "open_nvim",
  "new_branch_worktree",
  "existing_branch_worktree",
  "link_current",
  "unlink_repo",
  "capture",
  "delete_worktree",
  "done",
  "pause",
  "archive",
  "copy_cd",
  "resume",
  "abandon",
]
disabled = []

[shortcuts]
new_journey = "ctrl+n"
open_claude = "c"
open_nvim = "n"
new_branch_worktree = "b"
existing_branch_worktree = "w"
link_current = "l"
unlink_repo = "u"
capture = "t"
delete_worktree = "d"
done = "f"
pause = "p"
archive = "x"
copy_cd = "none"
resume = "none"
abandon = "none"
insert_mode = "a"
normal_mode = "esc"
```

</details>

Shortcuts accept single keys (`c`), control keys (`ctrl+n`), `esc`, or `none` for unbound actions. Duplicate shortcuts are rejected on startup.

## Upcoming Features

- Richer worktree navigation (`journey worktree list`, `journey cd <name>`, `worktrees/<repo>@<branch>` naming)
- Extended document previews and tabs
- User-defined shell scripts as custom interactive actions

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch (`git checkout -b my-feature`)
3. Make your changes and add tests where appropriate
4. Run `cargo test` to verify
5. Open a pull request

For bugs and feature requests, please [open an issue](https://github.com/yon1nja/journey/issues).

## License

[MIT](LICENSE) — see [LICENSE](LICENSE) for details.
