# Journey

Journey is a local context container for engineering work. It gives one effort a durable folder on disk with metadata, a short description, linked git worktrees, effort-local docs, and lifecycle status.

Journey is intentionally not a task manager, checkpoint system, agent orchestrator, or replacement for git. It preserves context around the work while leaving notes, plans, decisions, and version control to the tools and files you already use.

> **Active development notice**
>
> Journey is under active development and may contain bugs, rough edges, or behavior that changes between releases. Issue reports, bug reports, and feedback are very welcome.

## Philosophy

Journey's philosophy is to provide the smallest durable container for an engineering effort while staying deliberately unopinionated about the workflow inside it.

It should help manage simultaneous streams of engineering work under clear context umbrellas: each Journey answers what the effort is, why it exists, where its worktrees and notes live, and what lifecycle state it is in. Beyond that, Journey should stay extensible and quiet. It should not decide how you plan, document, delegate, summarize, or reason about the work.

Configuration is intentionally simple today: action order, disabled actions, and keyboard shortcuts. That is a starting point, not the ceiling. Journey should evolve toward deeper flexibility and configurability while keeping the core model small and durable.

## Installation

From source:

```sh
cargo install --path .
```

For development:

```sh
cargo build
cargo test
cargo run -- --help
```

Persistent state defaults to `~/.journey`. Set `JOURNEY_HOME` to use a different state directory.

## Core Concepts

A Journey represents one engineering effort, such as an investigation, migration, review, or feature implementation. A Journey can span multiple repositories and multiple live git worktrees.

Journey stores:

- `journey.yaml`: the Journey id, title, optional description, status, creation time, and linked repos.
- `journal.jsonl`: an append-only operational log for link, unlink, and status changes.
- `AGENTS.md`: a short pointer to the shared Journey agent guidance.
- `CLAUDE.md`: a Claude-specific pointer to `AGENTS.md`.
- `README.md`: an optional top-level overview for the Journey.
- `docs/`: optional user-owned Markdown files.
- `worktrees/`: symlinks to attached git worktrees.

Global state under `JOURNEY_HOME` includes:

- `JOURNEY-AGENTS.md`: shared coding-agent guidance for all Journeys under this home.
- `index.yaml`: the Journey registry used by list and TUI views.
- `worktree-index.yaml`: ownership mapping from canonical worktree paths to active or paused Journeys.
- `config.toml`: interactive action ordering and shortcuts.

## Quick Start

Create a Journey:

```sh
journey new "Investigate auth failures" --description "Reproduce and fix intermittent login failures"
```

Link the current repo or worktree:

```sh
journey link .
```

Capture a note:

```sh
journey capture "Token refresh fails after idle timeout"
```

See the current Journey summary:

```sh
journey status
```

Pause and resume the effort:

```sh
journey pause
journey resume
```

## Using Journey

Running `journey` opens the interactive terminal interface. This is the most complete way to use Journey: it brings together browsing, searching, details, document viewing, creation, and actions in one place.

The CLI subcommands expose the same core model for scripts, tests, automation, and agent workflows.

### Interactive Interface

The terminal interface includes:

- **Modes**: normal mode for navigation and actions, insert mode for search input.
- **Search**: filter Journeys by title, id, description, status, and linked repo names.
- **Details pane**: inspect metadata, lifecycle state, linked repos, recent events, and paths.
- **Document viewing**: read `README.md` and `docs/*.md` from inside the Details pane.
- **Document tabs**: switch between available Journey docs without leaving the interface.
- **Creating Journeys**: create a new Journey from inside the app with `Ctrl-N`.
- **Actions**: run configured Journey actions such as capture, link, unlink, pause, archive, resume, open editor, or create worktrees.

## Context Resolution

Commands that operate on an existing Journey resolve context in this order:

1. An explicit id from a positional argument or `--journey <id>`.
2. Walking up from the current directory until `journey.yaml` is found.
3. Matching the current directory against `worktree-index.yaml`.
4. Failing with an error if no Journey can be found.

This means `journey status`, `journey capture`, and doc commands can run from inside a Journey folder or from inside an attached worktree without repeating the Journey id.

## Journey Lifecycle

A Journey has one of four statuses:

- `active`: current or ongoing work. Active Journeys can own linked worktrees.
- `paused`: work that is not currently active but should remain resumable. Paused Journeys also keep ownership of linked worktrees.
- `archived`: completed or closed work that should remain available for reference. Archiving releases worktree ownership from the global index.
- `abandoned`: stopped work that should remain available for reference but is no longer expected to continue. Abandoning also releases worktree ownership.

Worktree ownership matters because Journey uses `worktree-index.yaml` to resolve context from inside attached worktrees. A canonical worktree path can belong to only one active or paused Journey at a time.

When a Journey is archived or abandoned, Journey detaches its worktrees from the global ownership index. It does not remove the git worktree directories from disk. The Journey still records the repo references in `journey.yaml`, and the Journey folder remains on disk, but those worktrees are no longer reserved for that Journey.

When an archived or abandoned Journey is resumed, Journey attempts to reattach its recorded worktrees. Reattachment can fail if one of those worktrees is missing or already owned by another active or paused Journey.

Lifecycle commands do not snapshot, stash, restore, delete, or compare code. Git remains responsible for code state. The only exception is the interactive `done` action, which archives the Journey and removes linked git worktrees after confirmation.

## Commands

### `journey`

With no subcommand, Journey opens the interactive terminal app when stdout is a terminal. In non-interactive contexts it prints the active Journey list.

The app has:

- a Journey list with active, paused, archived, abandoned, and all filters;
- a Details pane with metadata, linked repos, recent events, docs, and rendered Markdown preview;
- an Actions pane driven by configurable actions and shortcuts;
- dialogs for capture, linking, unlinking, worktree creation, worktree deletion, and completion.

Default navigation:

- `a`: enter insert/search mode.
- `Esc`: return to normal mode, back out of panes, or quit.
- `Ctrl-N`: create a Journey.
- `Enter`: open details or run the selected action.
- `Tab` / `Shift-Tab`: switch document tabs in Details.
- `Up` / `Down` / `PageUp` / `PageDown`: navigate or scroll.

### `journey new`

```sh
journey new <title> [--description <text>]
```

Creates a Journey without opening the interactive app. The id is derived from the title and made unique if needed.

New Journeys include `journey.yaml`, `journal.jsonl`, `AGENTS.md`, and `CLAUDE.md`. `AGENTS.md` points coding agents to the shared `JOURNEY_HOME/JOURNEY-AGENTS.md` guidance, and `CLAUDE.md` references `AGENTS.md`.

### `journey list`

```sh
journey list [--status active|paused|archived|abandoned] [--non-interactive]
```

Lists Journeys. In a terminal, this opens the interactive app with the selected initial status filter. With `--non-interactive`, it prints tab-separated rows:

```text
<id>    <status>    <updated>    <repos>
```

### `journey status`

```sh
journey status [<id>]
```

Prints a one-screen summary: title, description if present, id, status, path, repo count, and event count.

### `journey link`

```sh
journey link <repo-path> [--name <name>] [--journey <id>]
```

Links a git repo or worktree to a Journey. Journey records the canonical worktree path, current branch, and repo name. It also creates a symlink under the Journey's `worktrees/` directory.

A canonical worktree can belong to only one active or paused Journey. Archived and abandoned Journeys release worktree ownership.

### `journey unlink`

```sh
journey unlink <repo-name> [--journey <id>]
```

Detaches a linked repo from the Journey and removes its `worktrees/` symlink. This does not delete the git checkout.

### Lifecycle Commands

```sh
journey resume [<id>]
journey pause [<id>]
journey archive [<id>]
journey abandon [<id>]
```

`resume` marks a Journey active. `pause` marks it paused. `archive` and `abandon` mark terminal statuses and detach worktree ownership from the global index.

Lifecycle commands do not snapshot, stash, restore, or compare code. Git remains responsible for code state.

### `journey capture`

```sh
journey capture [--journey <id>] [--doc <name>] <text>
```

Appends a timestamped Markdown entry to `docs/capture.md` by default. If no text arguments are passed, Journey reads capture text from stdin.

Examples:

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

Creates, lists, or prints paths for Journey-local Markdown docs under `docs/`. Doc names are filenames, not paths; absolute paths and path separators are rejected.

Journey never overwrites existing docs.

### `journey readme`

```sh
journey readme new [--journey <id>]
journey readme path [--journey <id>]
```

Creates or prints the path to the optional top-level Journey `README.md`. The create command refuses to overwrite an existing README.

### `journey doctor`

```sh
journey doctor [--repair]
```

Checks `worktree-index.yaml` for missing worktrees, missing Journeys, and mismatches between the global index and Journey files.

With `--repair`, Journey rebuilds the worktree index from active and paused Journeys.

## Interactive Actions

The Actions pane exposes these operations:

- `open_claude`: leave the app and run `claude` in the Journey folder.
- `open_nvim`: leave the app and run `nvim` in the Journey folder.
- `new_branch_worktree`: create a new branch and git worktree, then link it.
- `existing_branch_worktree`: select an existing branch, create or reuse a worktree, then link it.
- `link_current`: link the current working directory's git worktree.
- `unlink_repo`: detach a linked repo without deleting it.
- `capture`: append a timestamped note to `docs/capture.md`.
- `delete_worktree`: remove a linked git worktree and unlink it. Journey refuses to delete the main worktree or a worktree with uncommitted changes.
- `done`: archive the Journey and remove linked worktrees after confirmation.
- `pause`: mark paused.
- `archive`: mark archived and release worktree ownership.
- `resume`: mark active and reattach worktree ownership when possible.
- `abandon`: mark abandoned and release worktree ownership.
- `copy_cd`: copy a `cd <journey-path>` command to the clipboard.

## Configuration

Journey creates `config.toml` in `JOURNEY_HOME` on first use.

Default config:

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

Shortcuts accept single keys such as `c`, control keys such as `ctrl+n`, `esc`, or `none` for unbound actions. Journey validates duplicate shortcuts on startup.

## Files and Ownership

Journey-owned structured files:

- `journey.yaml`
- `journal.jsonl`
- Journey `AGENTS.md`
- Journey `CLAUDE.md`
- global `JOURNEY-AGENTS.md`
- global `index.yaml`
- global `worktree-index.yaml`
- global `config.toml`

User-owned files:

- Journey `README.md`
- `docs/*.md`
- custom files or folders inside a Journey

Do not edit Journey-owned structured files directly unless you are repairing state by hand. Use CLI commands for status changes, linking, unlinking, and docs.

## Upcoming Features

Planned features from the current Journey docs:

- Better worktree navigation, such as `journey worktree list`, `journey cd <name>`, or richer symlink naming like `worktrees/<repo>@<branch>`.
- Continued improvements to document previews and tabs in the Details pane.
- Extended action configuration, including user-defined shell or bash scripts that can appear as Journey actions in the interactive interface.

The current implementation already creates `CLAUDE.md` for new Journeys and includes document tabs in the Details pane.

## Homebrew Publishing Notes

For Homebrew, Journey should be shipped as a source-built Rust formula with a stable tagged release tarball.

A private tap is the fastest publishing path. Homebrew taps are external Git repositories, and GitHub-hosted taps are conventionally named `homebrew-<name>`. A tap formula can live under `Formula/`, `HomebrewFormula/`, or the tap root, with `Formula/` recommended for organization.

For `homebrew/core`, Journey needs a stable tagged version, homepage, SPDX license, source tarball checksum, formula test, and passing `brew audit --new --formula journey`. Because `homebrew/core` applies notability and self-submission requirements, early releases should probably start in a project tap.

See `docs/homebrew-publishing.md` for the publish checklist and example formula skeleton.
