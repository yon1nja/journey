# AGENTS.md

This file gives coding agents the current project context for Journey.

## Product Shape

Journey is a local context container for engineering efforts. It is intentionally minimalist:

- It stores effort metadata, a short description, linked live worktrees, human-authored docs, scratch files, and lifecycle status.
- It does not manage tasks, decisions, questions, next actions, generated handoff files, checkpoints, or internal code versions.
- Git remains responsible for version control. Journey shells out to the `git` CLI where git behavior is needed.

The core invariant is that Journey owns the local container and indexes, not the user's workflow.

## Current User Experience

- `journey` with no subcommand opens the full-screen terminal starter.
- `journey new <title> --description <text>` creates a Journey without interactive UI and is the right path for scripts and agents.
- `journey list` uses the real `fzf` binary in interactive terminals. It shows Journey names in the list and a preview pane with details.
- `journey list --non-interactive` prints table output for scripts and agents.
- `journey status [<id>]` prints a compact summary.
- `journey doc new/list/path` are convenience helpers for user-owned Markdown files under `docs/`.
- `journey link` and `journey unlink` attach and detach live git worktrees by reference.
- `journey resume`, `pause`, `archive`, and `abandon` are lifecycle status changes only.
- `journey doctor [--repair]` inspects or rebuilds worktree attachment indexes.

## Important Invariants

- Do not reintroduce checkpoints, snapshots, git stash workflows, dirty-state capture, restore/apply commands, or internal version management.
- Do not reintroduce `NOW.md` or generated projection files.
- Do not reintroduce `ask`, `decide`, `resolve`, `next`, `note`, or structured narrative workflow commands.
- Docs and specs belong to the user. Journey may create `docs/<name>.md`, but it must not overwrite user docs or prescribe their contents beyond the initial heading.
- Worktrees are referenced, not owned. The `worktrees/` directory contains symlinks as a convenience view only.
- A canonical worktree path can be attached to only one active or paused Journey at a time.
- Archived and abandoned Journeys release worktree ownership. Active and paused Journeys may own worktrees.
- Lifecycle commands do not modify code state.
- Context resolution should work from a Journey folder or from inside an attached worktree.

## Data Model

Default state root is `~/.journey`; tests and agents can set `JOURNEY_HOME`.

```text
~/.journey/
|-- index.yaml
|-- worktree-index.yaml
`-- journeys/
    `-- <journey-id>/
        |-- journey.yaml
        |-- journal.jsonl
        |-- docs/
        `-- worktrees/
```

Key files:

- `journey.yaml`: id, title, optional description, status, created timestamp, linked repos.
- `index.yaml`: global registry used by list/discovery.
- `worktree-index.yaml`: canonical worktree path to owning Journey id and repo name.
- `journal.jsonl`: operational events only. Current produced events are `link_repo`, `unlink_repo`, and `status_change`.

Environment variables:

- `JOURNEY_HOME`: override state directory.
- `JOURNEY_SESSION`: override journal session id.

## Context Resolution

Commands requiring a Journey should resolve context in this order:

1. Explicit Journey id when provided.
2. Walk upward from the current directory looking for `journey.yaml`.
3. Canonicalize the current directory and find the nearest owning worktree in `worktree-index.yaml`.
4. Fail with a clear message asking for an explicit Journey id.

Keep this behavior when adding commands that operate on an existing Journey.

## UI Direction

The terminal UI should stay aligned with the current `journey list` style:

- cyan active labels
- dim metadata labels
- green success state
- left list or steps pane
- right preview/details pane
- prompts at the bottom

`journey list` should continue using real `fzf`. The no-subcommand starter currently uses terminal rendering and line prompts without an additional TUI framework.

## Code Map

- `src/cli.rs`: clap command definitions.
- `src/app.rs`: command dispatch, core command behavior, starter TUI rendering.
- `src/storage.rs`: files, indexes, context resolution, symlinks, atomic writes.
- `src/models.rs`: serialized data structures.
- `src/events.rs`: journal read/write and timestamps.
- `src/git.rs`: git CLI wrapper and worktree creation.
- `src/picker.rs`: fzf list, preview, and action popup.
- `tests/cli_flow.rs`: end-to-end CLI behavior.

## Development Notes

- Prefer existing patterns and keep the surface small.
- Use `apply_patch` for manual edits.
- Use `rg` for searching.
- Be careful with dirty worktrees. Do not revert unrelated changes.
- Avoid destructive git commands unless explicitly requested.
- If adding dependencies, justify why the existing standard library, `console`, `clap`, and git/fzf CLIs are not enough.

## Verification

For code changes, run:

```sh
cargo fmt
cargo test
cargo clippy -- -D warnings
```

For CLI surface changes, also check relevant help output, for example:

```sh
cargo run -- --help
cargo run -- list --help
cargo run -- doc --help
```

For interactive UI changes, use a pseudo-terminal smoke test with a temporary `JOURNEY_HOME` so the real user state is not modified.
