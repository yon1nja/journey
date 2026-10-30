# Journey: guide for agents

This guide is printed by `journey guide` and kept in sync at `~/.journey/JOURNEY-AGENTS.md`. It always matches the installed `journey` binary.

## What Journey is

Journey is durable, file-based memory for engineering work. You, the agent, operate it on the user's behalf.

Your session is short-lived: your context fills up, gets compacted, and ends. The work is not: it spans days or weeks, several sessions, several agents, and several repos. A journey is a folder on disk where the context of that work survives, so that any agent in any later session can pick the work up and continue it.

The user directs; you do the work. Users rarely open journeys themselves. They tell you "start a journey about X", "open a journey for this and record everything", or "continue the X work", and expect you to handle the rest.

## When to use it

- **The user asks you to start a journey.** Create it, record the goal, then work and record as you go.
- **The user asks you to open a journey mid-session.** Create it, then write down what this session already knows before it is lost to compaction or the session ending.
- **The user asks you to continue some work.** Find the journey and resume it (see below).
- **Work will clearly outlast this session and no journey exists for it.** Suggest starting one. Don't create journeys the user didn't ask for.

## Starting a journey

First check that one doesn't already exist: `journey list --search "<words>"`. Then:

```sh
journey new "<title>" --description "<what this effort is and why it exists>"
```

The output names the new journey's id and folder. Then:

1. Link the code it works on: `journey link <repo-or-worktree-path>`, or create a fresh branch worktree with `journey worktree new --branch <name> --repo <path>`.
2. Record the context in the journey folder: what the effort is, what you know, where it stands. A `README.md` is the usual overview (`journey readme new` creates it). Other notes go in `docs/` (`journey doc new <name>`) or wherever the journey's conventions say.

A good description says what the effort is and why it exists in a sentence or two. It is not a status log.

## Recording

What goes inside a journey belongs to the user, and Journey prescribes no structure. Journeys may be run very differently: one keeps task files and a roadmap, another a single running log, another the plans of a workflow tool.

- Before writing, learn how this journey is run: read its `README.md`, its `AGENTS.md`, and the file list in `docs/`. Follow the conventions you find.
- If the journey has no conventions yet, pick a structure that fits the work and stay consistent with it.
- Write down what a future agent needs to continue without this session: the goal, decisions and why they were made, findings, current state, and next steps.
- Record as you go, not only at the end. A session can end at any moment.
- Don't delete or overwrite content you didn't write unless the user asks.

## Resuming

```sh
journey list --search "<words>"   # is there already a journey for this? (every status)
journey list                      # active journeys: id, status, updated, repos
journey list --status paused      # or done, archived, abandoned; --all for every status
journey status <id>               # summary: path, README, docs, repos and branches
```

Then go to the journey folder and read how it is run (as above) before continuing. Commands also resolve the journey automatically when run from inside its folder or a linked worktree.

## Closing

When the effort is finished, close it with a one-line outcome:

```sh
journey close <id> --outcome "<how it ended>"
```

This marks it `done` and releases its worktrees (they stay on disk unless you pass `--remove-worktrees`). Other lifecycle commands: `journey pause`, `journey resume`, `journey archive` (shelved), `journey abandon` (stopped).

## Rules

- Don't edit `journey.yaml` or `journal.jsonl` directly. Use the CLI.
- Git owns code. Journey never snapshots, stashes, or restores code state, and neither should you on its behalf.
- Worktrees are referenced, not owned. `worktrees/` contains symlinks to them.
- A worktree can be linked to only one open (active or paused) journey at a time.
- Journey is not a task manager and doesn't track tasks. Any task files you keep are ordinary documents.

## Command reference

| Command | What it does |
|---|---|
| `journey new <title> [--description <text>]` | Create a journey |
| `journey list [--status <s> \| --all] [--search <text>]` | List journeys (default: active) or search them |
| `journey status [<id>]` | Show a journey's summary |
| `journey link <path> [--name <name>]` | Link an existing repo or worktree |
| `journey unlink <name>` | Unlink a repo or worktree (leaves it on disk) |
| `journey worktree new --branch <b> [--from <start>] [--repo <path>]` | Create or reuse a branch worktree and link it |
| `journey worktree remove <name>` | Delete a linked worktree from disk and unlink it |
| `journey readme new` / `journey readme path` | Create or locate `README.md` |
| `journey doc new <name>` / `list` / `path <name>` | Create, list, or locate docs under `docs/` |
| `journey capture <text> [--doc <name>]` | Append a timestamped note to a doc (default `docs/capture.md`) |
| `journey close [<id>] --outcome <text>` | Finish a journey with an outcome |
| `journey pause` / `resume` / `archive` / `abandon` `[<id>]` | Other lifecycle changes |
| `journey guide` | Print this guide |

`new`, `list`, `status` and `doc list` accept `--json`, which includes each journey's folder path. Prefer it when you need to parse output.

Commands that act on one journey take `--journey <id>` (or a positional id for lifecycle commands). Without it, they resolve the journey from the current directory: the journey folder itself, or a linked worktree.
