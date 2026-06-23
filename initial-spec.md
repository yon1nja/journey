# Journey - Specification

> A local container for an engineering effort.

Journey is a Rust CLI for preserving the working context of an engineering effort. It keeps the effort's folder, metadata, short description, linked worktrees, and human-authored files together without prescribing how the developer or agent should write notes, make decisions, or manage tasks.

The design bet: the unit of organization is the effort, not the ticket, branch, repo, chat, or document. Journey owns the lightweight local container around that effort.

---

## 1. Problem

Engineering work is interrupted, spans multiple repositories, and often includes scratch docs, plans, AI conversations, and temporary worktrees. Git preserves code, but it does not answer: which live checkouts belong to this effort, why did this effort exist, and where are the effort-local files?

Journey is the missing local folder and index for that effort.

## 2. Non-goals

Journey is not:

- a task manager
- a decision tracker
- a question tracker
- a knowledge base
- an agent orchestrator
- a server
- a replacement for git
- a generated documentation system
- an internal version management system

Journey does not provide `ask`, `decide`, `resolve`, `next`, generated `NOW.md`, or checkpoint workflows. Users and agents can keep whatever docs, thoughts, plans, or logs they want inside the Journey folder.

## 3. Core Model

**3.1 The effort is the unit.** A Journey is one effort: "investigate prod auth failures", "review PR-1234", "migrate service to new API". It may span multiple repos and worktrees.

**3.2 Journey starts from where you are.** Running `journey` with no subcommand opens a full-screen terminal starter in the current directory. It uses the same visual language as `journey list`: steps on the left, a preview pane on the right, colored labels, and prompts at the bottom.

**3.3 Descriptions are first-class hints.** A Journey can store an optional short description. The title says what the effort is; the description explains why it was opened or what future readers should remember.

**3.4 Journey is a container, not a workflow.** A Journey folder can hold docs, specs, thoughts, scratch files, and links to worktrees. Journey does not prescribe the shape of those files.

**3.5 Config and indexes stay explicit.** `journey.yaml`, the global `index.yaml`, and `worktree-index.yaml` are the structured core. They make discovery, lifecycle, and context resolution cheap.

**3.6 Lifecycle is status only.** `pause`, `resume`, `archive`, and `abandon` update Journey status. They do not capture, restore, or compare code state.

**3.7 Docs are human-owned.** `docs/` is a convenience location for effort-local Markdown. It is not generated and not required.

## 4. Design Principles

1. **Minimal product surface.** Provide the effort container and indexes. Avoid opinionated narrative structures.
2. **Reference worktrees, do not own them.** Journey records live worktree paths and creates symlinks under `worktrees/` as a convenience view.
3. **One owning Journey per worktree.** A canonical worktree path can be attached to only one active or paused Journey at a time.
4. **No internal versioning.** Git remains responsible for version control. Journey does not capture or restore code versions.
5. **Keep files local.** No daemon, no server, no account, no external service.
6. **Let workflows vary.** A user may keep a plan in `docs/`, raw notes in a custom folder, or nothing but worktree links.

## 5. Anatomy of a Journey

The scaffold is lazy. `journey.yaml` and `journal.jsonl` are created immediately; other folders appear when used.

```text
~/.journey/
|-- index.yaml                  # global registry of journeys
|-- worktree-index.yaml         # ownership index for linked worktrees
`-- journeys/
    `-- prod-auth-failures/
        |-- journey.yaml        # metadata + linked repos
        |-- journal.jsonl       # operational event log
        |-- docs/               # optional human-authored docs
        |   |-- investigation.md
        |   `-- migration-plan.md
        `-- worktrees/          # optional symlink index of linked worktrees
            |-- frontend -> /home/yh/src/web-auth
            `-- backend -> /home/yh/src/api-auth
```

There is no generated `NOW.md`. Any summary, plan, or handoff document is a user-owned file.

## 6. Data Formats

### 6.1 `journey.yaml`

```yaml
id: prod-auth-failures
title: Investigate production authentication failures
description: Reproduce and fix intermittent auth failures seen under production load
status: active
created: 2026-06-20T09:14:03Z
repos:
  - name: frontend
    root: /home/yh/src/web-auth
    worktree: /home/yh/src/web-auth
    branch: fix/auth-token-refresh
  - name: backend
    root: /home/yh/src/api-auth
    worktree: /home/yh/src/api-auth
    branch: fix/session-validation
```

`description` is optional. It is shown in `journey status` and the interactive list preview.

`repos` is the cross-repo index. It records references to live worktrees; it does not own or version their contents. In the current implementation, `root` is the discovered git worktree top-level path for the linked checkout.

### 6.2 `index.yaml`

`index.yaml` is the global Journey registry:

```yaml
journeys:
  - id: prod-auth-failures
    title: Investigate production authentication failures
    description: Reproduce and fix intermittent auth failures seen under production load
    status: active
    updated: 2026-06-20T09:14:03Z
    repos:
      - frontend
      - backend
```

The interactive and non-interactive list commands read this index.

### 6.3 `worktree-index.yaml`

`worktree-index.yaml` maps canonical live worktree paths to the one active or paused Journey that currently owns them for context resolution:

```yaml
attachments:
  - worktree: /home/yh/src/web-auth
    journey_id: prod-auth-failures
    repo_name: frontend
    attached_at: 2026-06-20T09:14:03Z
```

A worktree can be attached to only one active or paused Journey at a time. `journey link` fails if the canonical worktree path is already attached.

### 6.4 `journal.jsonl`

The journal is an operational log, not a structured narrative system. Current event types are:

- `link_repo`
- `unlink_repo`
- `status_change`

Example:

```jsonc
{"seq":1,"ts":"...","session":"yh","type":"link_repo","name":"frontend","root":"...","worktree":"...","branch":"fix/auth-token-refresh"}
{"seq":2,"ts":"...","session":"yh","type":"status_change","status":"paused"}
```

Older event variants may still be readable for backward compatibility, but the CLI no longer produces or displays them.

### 6.5 Journey Docs

`docs/` contains optional human-authored Markdown files for the effort. They are edited directly and never overwritten by Journey.

The CLI has small helpers:

- `journey doc new <name>`
- `journey doc list`
- `journey doc path <name>`

Doc names are single filenames under `docs/`. The CLI rejects absolute paths and path separators.

These helpers are convenience only. Users can create any other folders or files inside the Journey directory.

## 7. Commands

### Context Resolution

Commands that require an existing Journey resolve context this way:

1. Use an explicit id when supplied through a positional id or `--journey`.
2. Walk upward from the current directory looking for `journey.yaml`.
3. Canonicalize the current directory and look for the nearest owning worktree in `worktree-index.yaml`.
4. If none match, fail and ask for an explicit Journey id.

This means commands such as `journey doc list`, `journey pause`, or `journey status` can run from inside a Journey folder or inside an attached worktree without repeating the Journey id.

### Command Surface

```text
journey                                      # full-screen terminal starter from the current folder
journey new <title> [--description <text>]  # automation-friendly Journey creation
journey link <repo-path> [--name <name>] [--journey <id>]
                                             # link a repo/worktree by reference
journey unlink <repo-name> [--journey <id>]  # release a linked repo/worktree
journey resume [<id>]                       # mark a Journey active
journey pause [<id>]                        # mark a Journey paused
journey archive [<id>]                      # mark archived and release worktree ownership
journey abandon [<id>]                      # mark abandoned and release worktree ownership
journey doc new <name> [--journey <id>]     # create docs/<name>.md
journey doc list [--journey <id>]           # list Journey docs
journey doc path <name> [--journey <id>]    # print an absolute doc path
journey list [--status active]              # fzf discovery with an editable default filter
journey list --non-interactive              # table output for scripts/agents
journey status [<id>]                       # compact Journey summary
journey doctor [--repair]                   # inspect or rebuild worktree index
```

### Bare `journey` Starter

`journey` with no subcommand requires an interactive terminal. It renders a full-screen terminal UI with:

- a left steps pane: Details, Worktrees, Create, Done
- a right preview pane with current folder, git root, branch, title, description, id, path, linked worktrees, and recent activity
- cyan active labels, dim metadata labels, green completion state
- prompts at the bottom

When the current directory is inside a git repo, the starter offers:

1. Attach current worktree.
2. Create and attach one new worktree.
3. Create and attach multiple new worktrees.
4. Do not attach a worktree now.

New worktrees are created through the git CLI. The default worktree path is based on the repo name and Journey slug, and the default branch name is the Journey slug.

In non-interactive contexts, scripts and agents should use `journey new <title> --description <text>` and then `journey link` as needed.

### `journey list`

`journey list` uses the real `fzf` binary in a TTY. The fzf source contains all Journeys, starts with an editable `active` query by default, and shows only Journey names in the list. Clearing the query displays all Journeys.

The preview shows id, description, status, updated time, path, repos, and docs. The search haystack includes id, title, description, status, and repo names.

Enter opens a nested fzf action popup. Current actions:

- resume
- status
- open shell in Journey folder
- print Journey path
- pause
- archive
- abandon

In non-interactive mode, `--status` is a hard status filter and the command prints tab-separated table output.

### Lifecycle Commands

- `resume` marks a Journey `active`.
- `pause` marks a Journey `paused`.
- `archive` marks a Journey `archived` and detaches all worktree ownership from `worktree-index.yaml`.
- `abandon` marks a Journey `abandoned` and detaches all worktree ownership from `worktree-index.yaml`.

Lifecycle commands do not restore, snapshot, stash, or compare code. Re-activating an archived or abandoned Journey attempts to reattach its worktrees and fails if any are now owned by another active or paused Journey.

### Doctor

`journey doctor` inspects `worktree-index.yaml` for missing worktrees, missing Journeys, and mismatches between the index and Journey files.

`journey doctor --repair` rebuilds the worktree index from active and paused Journeys, skipping missing worktree paths and failing on duplicate ownership.

## 8. Runtime and Dependencies

- The CLI is written in Rust.
- Persistent state defaults to `~/.journey`.
- `JOURNEY_HOME` overrides the state directory and is used by tests.
- `JOURNEY_SESSION` overrides the journal session id; otherwise the CLI uses `USER` or `local`.
- Git integration shells out to the `git` CLI.
- Interactive `journey list` requires the external `fzf` binary.
- The bare `journey` starter is implemented with terminal output and prompts, not a separate TUI framework.

## 9. Scope

**In:** Rust CLI; interactive `journey` starter; `new`, `link`, `unlink`, `resume`, `pause`, `archive`, `abandon`, `list`, `status`, `doc new/list/path`, `doctor`; optional Journey descriptions; `journey.yaml`; `journal.jsonl`; global index; worktree ownership index; Journey-local docs; symlinked `worktrees/`; fzf list UI.

**Out:** generated `NOW.md`; checkpoints; code snapshots; code restore/apply; `ask`; `decide`; `resolve`; `next`; task management; structured note management; multi-machine sync; web UI; server mode.

## 10. Open Questions

1. **Multi-machine.** Absolute worktree paths do not travel. Export/sync would need a separate design.
2. **Generic files.** Should Journey offer helpers for arbitrary files/folders beyond `docs/`, or should the folder remain fully user-managed?
3. **Richer starter UI.** The current starter follows the `journey list` visual style without adding a TUI framework. A future version could use a richer terminal UI crate if form editing, validation, and navigation become complex enough to justify the dependency.
