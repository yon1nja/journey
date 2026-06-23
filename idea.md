Journey

Journey is a local context container for AI-assisted engineering.

It gives a single engineering effort a durable home on disk: metadata, a short description, linked live worktrees, effort-local docs, scratch files, and lifecycle status. It does not try to be a task manager, decision tracker, question tracker, note-taking system, generated handoff document, or internal version manager.

Core Concept

A Journey represents one engineering effort:

* Investigate production authentication failures
* Design EDL package publishing
* Implement new editor architecture
* Review PR-1234
* Migrate a service to a new API

Each Journey can contain or reference what is needed to return to that effort:

* Journey metadata: id, title, optional description, status, creation time
* Linked live worktrees across one or more git repositories
* Draft docs and specs under the Journey folder
* Scratch notes or custom folders created by the user
* Lifecycle status: active, paused, archived, abandoned

The goal is that after hours, days, or weeks away from a problem, a developer can find the effort, inspect its folder and linked worktrees, and continue using their own workflow.

Primary Experience

Running `journey` with no subcommand starts the full-screen terminal starter from the current folder. It uses the same pane-and-preview visual language as `journey list`: steps on the left, current context and draft metadata on the right, colored labels, and prompts at the bottom.

The starter:

* detects the current folder
* detects the current git repo and branch when available
* asks for a Journey title
* asks for an optional short description explaining why the effort exists
* creates the Journey folder and metadata
* if inside a git repo, offers to attach the current worktree, create one new worktree, create multiple new worktrees, or skip worktree attachment

For automation and agents, `journey new <title> --description <text>` creates the same Journey metadata without opening the interactive starter.

Discovery Experience

`journey list` is interactive by default and uses the real `fzf` binary. It shows Journey names on the left and a preview pane on the right with the Journey id, description, status, path, linked repos, and docs. The default query is `active`, so active Journeys are shown first; clearing the query reveals all Journeys.

Pressing Enter opens an action popup inside fzf. Current actions include resume, status, open shell in the Journey folder, print path, pause, archive, and abandon.

Agents and scripts should use `journey list --non-interactive`, which prints table output and treats `--status` as a hard filter.

Philosophy

Journey is intentionally minimalist and local.

It should answer:

* What effort is this?
* Why was this effort opened?
* Is it active, paused, archived, or abandoned?
* Where is its folder?
* Which worktrees belong to it?
* What docs or scratch files live with it?

Journey should not decide how the user records thoughts, plans, decisions, questions, or next actions. Those belong in user-owned files, using whatever workflow makes sense for the effort.

Structure

Example:

```text
~/.journey/
|-- index.yaml
|-- worktree-index.yaml
`-- journeys/
    `-- error-investigation/
        |-- journey.yaml
        |-- journal.jsonl
        |-- docs/
        |   |-- investigation.md
        |   `-- migration-plan.md
        `-- worktrees/
            |-- frontend -> /actual/path/to/frontend-worktree
            `-- backend -> /actual/path/to/backend-worktree
```

`journey.yaml`, `index.yaml`, and `worktree-index.yaml` are the structured configuration and index layer.

`docs/` and any other user-created folders are human-owned. Journey does not generate `NOW.md`, capture code snapshots, or prescribe a documentation model.

Worktrees

Journey references live git worktrees; it does not own them. The `worktrees/` folder inside a Journey is a convenience view of symlinks.

`worktree-index.yaml` maps canonical worktree paths to the one active or paused Journey that currently owns them for context resolution. That means commands such as `journey doc list` can be run from inside an attached worktree without passing `--journey`.

Archived and abandoned Journeys release worktree ownership so those checkouts can be linked elsewhere. Paused Journeys keep ownership because they are still resumable context.

Design Principles

1. Context is the primary artifact.
2. Work is organized around efforts, not tickets or repos.
3. Journey owns the container, not the workflow.
4. Worktrees are referenced, not owned.
5. Each active or paused worktree belongs to at most one Journey.
6. Lifecycle commands change status only.
7. Git remains responsible for version control.
8. Everything is local files; no server is required.

Non-goals

Journey intentionally does not provide:

* checkpoints
* code snapshots
* code restore/apply
* generated `NOW.md`
* `ask`, `decide`, `resolve`, `next`, or structured note commands
* task management
* agent orchestration
* multi-machine sync
* a web UI or daemon

Vision

Journey should become the small missing layer between terminal workflows, AI agents, and effort-local working files. It should make complex work easy to pause and resume without requiring every user to adopt the same documentation discipline.
