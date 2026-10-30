# Journey

**Durable memory for engineering work, operated by your agents.**

[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Agent sessions are short-lived; engineering work is not. Journey gives each effort a durable folder on disk — what it is, why it exists, its linked git worktrees, its docs, and its lifecycle — so any agent, in any session, can pick the work up and continue it. You tell your agent "start a journey about X" or "open a journey for this and record everything"; the agent does the rest.

Journey is not a task manager, a workspace, or a replacement for git. It provides the container and stays out of what goes inside.

> [!NOTE]
> Journey is under active development and may contain bugs, rough edges, or behavior that changes between releases. Issue reports and feedback are very welcome.

---

## Table of Contents

- [Philosophy](#philosophy)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Core Concepts](#core-concepts)
- [Using Journey](#using-journey)
  - [Context Resolution](#context-resolution)
- [Journey Lifecycle](#journey-lifecycle)
- [Commands](#commands)
- [Contributing](#contributing)
- [License](#license)

---

## Philosophy

- **Agents are the primary users.** The CLI is the interface, and every capability works non-interactively.
- **Memory, not a workspace.** Your terminal, editor, and agent are the workspace. Journey holds the context.
- **Self-explaining.** Agents learn what Journey is and when to use it from the tool itself.
- **Unopinionated about what goes inside.** Each journey can be run its own way. Journey prescribes no document structure or workflow.
- **One effort, one context.** Unrelated work stays out of an effort's context.
- **Lifecycle matches the work.** Containers end when the work ends, and closing records an outcome.
- **Git owns code.** Worktrees are referenced, never snapshotted or owned.

The full philosophy is in [docs/philosophy.md](docs/philosophy.md).

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

For faster terminal access, consider adding a short alias to your shell profile:

```sh
alias jy='journey'
```

## Quick Start

### With an agent

Journey is meant to be driven by your coding agent. Agents learn it from the tool itself:

```sh
journey guide
```

prints what Journey is, when to start a journey, how to record and resume work, and the command reference. The same guide is kept current at `~/.journey/JOURNEY-AGENTS.md`, and every journey's `AGENTS.md` points to it.

Then just ask: "start a journey about the auth failures", "open a journey for this and record everything", or "continue the auth work".

### By hand

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
| `JOURNEY-AGENTS.md` | Agent guide, rewritten to match the installed binary |
| `index.yaml` | Journey registry used by `list` and discovery |
| `worktree-index.yaml` | Worktree-to-Journey ownership mapping |

## Using Journey

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
| `done` | Finished, closed with an outcome | No (released) |
| `archived` | Completed or closed, kept for reference | No (released) |
| `abandoned` | Stopped, not expected to continue | No (released) |

A canonical worktree path can belong to only one active or paused Journey at a time.

When a Journey is done, archived, or abandoned, its worktrees are detached from the global ownership index but **not deleted from disk**. The Journey folder and repo references remain intact.

When a done, archived, or abandoned Journey is resumed, Journey attempts to reattach its worktrees. Reattachment fails if a worktree is missing or already owned by another active/paused Journey.

Lifecycle commands never snapshot, stash, restore, or compare code — git remains responsible for code state. The one exception is `journey close --remove-worktrees`, which removes linked worktrees from disk after checking they have no uncommitted changes.

## Commands

### `journey` (no subcommand)

Prints a short overview and the active Journey list.

### `journey guide`

```sh
journey guide
```

Prints the agent guide embedded in the binary.

### `journey new`

```sh
journey new <title> [--description <text>]
```

Creates a Journey with `journey.yaml`, `journal.jsonl`, `AGENTS.md`, and `CLAUDE.md`.

### `journey list`

```sh
journey list [--status active|paused|done|archived|abandoned | --all] [--search <text>] [--json]
```

Lists Journeys with the given status (default: active) as tab-separated rows: `<id> <status> <updated> <repos>`. `--search` matches id, title, description, and repo names, case-insensitively, across every status unless `--status` is given.

### `journey status`

```sh
journey status [<id>]
```

Prints a summary: title, description, ID, status, outcome, path, README, docs, linked repos with branch and worktree, and event count.

### JSON output

`journey new`, `journey list`, `journey status`, and `journey doc list` accept `--json` for output agents and scripts can parse. JSON output includes each Journey's folder path.

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

### `journey close`

```sh
journey close [<id>] --outcome <text> [--remove-worktrees]
```

Marks a Journey `done` and records a one-line outcome in `journey.yaml` and the journal. The outcome is required. Worktree ownership is released; `--remove-worktrees` also removes the linked worktrees from disk.

### `journey worktree`

```sh
journey worktree new --branch <name> [--from <start>] [--repo <path>] [--path <dir>] [--name <name>] [--journey <id>]
journey worktree remove <name> [--journey <id>]
```

`new` creates a git worktree for a branch and links it. A missing branch is created from `--from` (default: HEAD). An existing branch that is already checked out somewhere is linked in place. The default location is `<repo>/.worktrees/<branch>`.

`remove` deletes a linked worktree from disk and unlinks it. It refuses the main working tree and worktrees with uncommitted changes.

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
