# AGENTS.md

You are working inside a Journey — a local context container for an engineering effort.

## This Journey

- **Config**: `journey.yaml` in this directory — contains id, title, description, status, and linked repos.
- **Journal**: `journal.jsonl` — append-only operational event log (link/unlink/status changes).
- **README**: `README.md` — optional user-owned top-level overview shown in Journey's interactive Details pane when present.
- **Docs**: `docs/` — user-owned Markdown files. You may create new docs here but never overwrite existing ones.
- **Worktrees**: `worktrees/` — symlinks to attached git worktrees (convenience view, not ownership).

Read `journey.yaml` first to understand what this effort is about.

## Journey CLI

The `journey` CLI manages this folder. Key commands:

- `journey status` — show this Journey's summary (run from this directory or any linked worktree).
- `journey link <repo-path>` — attach a git worktree to this Journey.
- `journey unlink <name>` — detach a worktree.
- `journey readme new` — create README.md if it does not already exist.
- `journey readme path` — print the absolute path to README.md.
- `journey doc new <name>` — create a new doc under `docs/`.
- `journey doc list` — list existing docs.
- `journey pause` / `journey resume` — lifecycle transitions.
- `journey list --non-interactive` — list all Journeys (for scripts/agents).

## Working with Config Files

- **`journey.yaml`**: YAML. Do not edit directly — use CLI commands to change status or link/unlink repos. You may read it freely to understand context.
- **`journal.jsonl`**: JSON Lines, append-only. Do not write to it directly — the CLI appends events automatically.
- **`README.md`**: User-owned Markdown. Use it for the Journey's top-level overview. Create it through `journey readme new` when missing; do not overwrite existing content.
- **`docs/*.md`**: You may create and edit these freely. Use them for investigation notes, plans, decisions, or any documentation relevant to the effort.

## Context Resolution

The CLI resolves which Journey you mean by:
1. Explicit `--journey <id>` flag.
2. Walking up from cwd looking for `journey.yaml`.
3. Matching cwd against the worktree index.
4. Failing with a clear error.

When working inside this folder or any linked worktree, commands resolve automatically.

## Constraints

- Journey is a context container, not a task manager. Do not create task-tracking files, checkpoints, or generated handoff documents.
- Git owns version control. Do not stash, snapshot, or capture dirty state through Journey.
- One worktree can be attached to only one active/paused Journey at a time.
