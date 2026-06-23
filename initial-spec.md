
# Journey — Specification

> Save/load for an engineering effort.

Journey is a Rust CLI and context-persistence layer for AI-assisted engineering. It snapshots the **environment and narrative of a single effort** so a developer or agent can leave for hours, days, or weeks and rebuild their working context in one command.

The design bet: the unit of organization is the *effort*, not the ticket, branch, or repo. Those are all the wrong granularity for "what was I doing and what's next." Nothing today owns that gap cleanly. Journey owns exactly that gap and leans on git for everything git already does.

---

## 1. Problem

Engineering work is constantly interrupted, spans multiple repos, and spawns multiple AI conversations. Git preserves code, Jira preserves tickets, Slack preserves conversation, Obsidian tracks tasks and knowledge — but nothing preserves *the working state of an effort and orchestrates all others*: which branch in which repo, what's uncommitted, the failing test command, the flag, the half-applied migration, the decision you made on Tuesday and the question still open.
 
The naive version of this — a folder of prose notes — fails for one reason: **staleness**. The instant `NOW.md` says "next: fix token refresh" when you did that two sessions ago, the human stops trusting it, re-derives from scratch, and the tool becomes dead weight. The entire design is shaped around defeating trust decay.

## 2. Non-goals

Journey is **not**:

- a task manager (no backlog, no assignees, no sprints)
- a knowledge base (effort-scoped and ephemeral, not a wiki, even though it might contain a lot of relevant docs)
- an agent orchestrator (it does not run, schedule, or route agents)
- a server (files on disk; no daemon, no account, no external service)
- a replacement for git (it invokes the git CLI, references git state, and symlinks worktrees for convenience; it does not own or re-implement git)

## 3. Core model

Five ideas carry the whole design.

**3.1 The effort is the unit.** A Journey is one effort — "investigate prod auth failures," "review PR-1234," "migrate service to new API." It may span several repos. It may contain a few different worktrees. It has a lifecycle and a status.

**3.2 The append-only log is the source of truth for structured context.** Notes, decisions, questions, next actions, commands, status changes, and checkpoints land in `journal.jsonl` as append-only events. Nothing is ever mutated in place. This keeps projections derivable, makes the history inspectable, and gives every claim a timestamp and author.

**3.3 Generated projections are not files you edit.** `NOW.md`, and later `DECISIONS.md` / `QUESTIONS.md`, are **rendered** from the log on every checkpoint and resume. They carry a `GENERATED — do not edit` banner. A hand-edit would be silently overwritten, so instead edits enter as events (`journey note`). Generated prose is for reading; the log is the trusted source for projected state.

**3.4 Journey-local docs are first-class.** Specs, design notes, investigation notes, migration plans, and other effort-local prose live under `docs/`. These are human-authored working documents, separate from generated projections, so they do not clutter repository roots before they are ready to be committed or published.

**3.5 Environment is captured precisely; narrative stays thin.** "What's next" reads like the hard problem but it's one sentence. What actually kills a cold resume is environment. So Journey captures branch / HEAD / upstream / tracked dirty state per linked repo as **machine-verifiable** data, and keeps the narrative deliberately lean. Prefer derivable state over asserted state everywhere it's possible.

## 4. Design principles

1. **Derivable over asserted.** If git can tell us, don't ask a human to write it down. Verifiable state can be checked against ground truth; freeform prose can't.
2. **Capture environment precisely, keep narrative thin.** The 80% of resume value is environment, not "what was I doing."
3. **Reference worktrees, don't own them.** Journey records `repo + path + branch + commit`; resume re-attaches. Journey points at working state, never contains it. A Journey-local `worktrees/` directory may contain symlinks as a convenient index of the live checkouts.
4. **Lean on git.** Branches, commits, reflog, worktrees, dirty snapshot objects, refs — all git. Journey adds only the non-redundant layer: the cross-repo effort index, the narrative/decision/question log, Journey-local docs, the worktree symlink index, and the resume action.
5. **Forcing function, not discipline.** Context-update must be a *side effect* of normal work, never optional housekeeping. Anything not in the log is invisible to resume — incentives aligned by construction.
6. **Journey-local docs by default.** Draft specs and effort notes belong in the Journey until there is an intentional decision to move them into a repo.
7. **Append-only for auditability.** The log is easier to trust and replay when events are added rather than rewritten. Heavy concurrent writing is not a v0 design center.
8. **Ceremony scales with the effort.** A 20-minute PR review must not cost the same scaffold as a 3-week migration. Files appear lazily.

## 5. Anatomy of a Journey

The full scaffold, but **created lazily** — only `journey.yaml` and `journal.jsonl` are mandatory; everything else appears the first time it's used.

```
~/.journey/
├── index.yaml                  # global registry of all journeys (discovery, lifecycle)
└── journeys/
    └── prod-auth-failures/
        ├── journey.yaml        # metadata + linked repos (by reference)   [required]
        ├── journal.jsonl       # append-only event log — source of truth  [required]
        ├── NOW.md              # projection: current state + next actions  [generated]
        ├── docs/               # human-authored journey docs and specs     [optional]
        │   ├── investigation.md
        │   └── migration-plan.md
        └── worktrees/          # symlink index of linked live worktrees     [optional]
            ├── frontend -> /home/yh/src/web-auth
            └── backend -> /home/yh/src/api-auth
```

Live worktrees live wherever git puts them, in the repos they belong to. Journey references them and creates a Journey-local `worktrees/` directory of symlinks so the developer can see the whole effort from one place. These symlinks are a convenience view, not ownership of the checkouts.

Human-authored Journey docs live under `docs/`. This is where draft specs, design notes, investigation notes, migration plans, and review notes belong while they are effort-local. Generated projections stay at the Journey root and are never hand-maintained.

## 6. Data formats

### 6.1 `journey.yaml`

```yaml
id: prod-auth-failures
title: Investigate production authentication failures
status: active            # active | paused | archived | abandoned
created: 2026-06-20T09:14:03Z
repos:
  - name: frontend
    root: /home/yh/src/web
    worktree: /home/yh/src/web-auth     # may equal root
    branch: fix/auth-token-refresh
  - name: backend
    root: /home/yh/src/api
    worktree: /home/yh/src/api-auth
    branch: fix/session-validation
```

`repos` is the cross-repo index — the thing no other tool gives you. Per-checkpoint commit/dirty data is *not* stored here (it changes constantly); it lives in checkpoint events.

### 6.2 Event log (`journal.jsonl`)

One JSON object per line, append-only. Every event carries ordering and provenance so projections are deterministic and attributable.

```jsonc
{"seq": 1,  "ts": "...", "session": "agent-3f2", "type": "link_repo",      "name": "frontend", "root": "...", "branch": "..."}
{"seq": 2,  "ts": "...", "session": "yh",        "type": "note",           "text": "Repro only under load; suspect token-refresh race."}
{"seq": 3,  "ts": "...", "session": "agent-3f2", "type": "question_open",  "qid": "q1", "text": "Is the refresh lock per-node or global?"}
{"seq": 4,  "ts": "...", "session": "agent-3f2", "type": "decision",       "did": "d1", "text": "Use a global Redis lock for refresh.", "because": "per-node allows concurrent refresh under LB"}
{"seq": 5,  "ts": "...", "session": "agent-3f2", "type": "checkpoint",     "message": "before switching to backend", "repos": [
    {"name": "frontend", "head": "a1b2c3d", "branch": "fix/auth-token-refresh", "upstream": "origin/...", "ahead": 2, "behind": 0, "tracked_dirty": true,  "dirty_snapshot_ref": "refs/journey/prod-auth-failures/5-frontend", "untracked_files": ["tmp/load-repro.log"]},
    {"name": "backend",  "head": "e4f5g6h", "branch": "fix/session-validation", "upstream": "origin/...", "ahead": 0, "behind": 1, "tracked_dirty": false, "dirty_snapshot_ref": null, "untracked_files": []}
]}
{"seq": 6,  "ts": "...", "session": "yh",        "type": "next_actions",   "items": ["Add integration test for concurrent refresh", "Wire Redis lock in backend"]}
{"seq": 7,  "ts": "...", "session": "agent-3f2", "type": "command",        "cmd": "pytest tests/auth -k refresh", "exit": 1, "cwd": "/home/yh/src/api-auth"}
{"seq": 8,  "ts": "...", "session": "yh",        "type": "question_resolve","qid": "q1", "answer": "Per-node today — that's the bug."}
```

Event types: `link_repo`, `note`, `decision`, `question_open`, `question_resolve`, `next_actions`, `command`, `checkpoint`, `status_change`. The set is small and extensible.

### 6.3 Projections

`NOW.md` is regenerated from the log by replaying events:

- **Current environment** — from the latest `checkpoint`: per repo, branch / HEAD / ahead-behind / tracked dirty state, untracked files, and a one-line `git diff --stat` of the dirty snapshot.
- **Next actions** — from the latest `next_actions` event, stamped with its age (see §10).
- **Open questions** — `question_open` minus `question_resolve`.
- **Recent decisions** — last N `decision` events.
- **Last commands** — recent `command` events with exit codes.

Because projection is a pure function of the log, two agents regenerating concurrently converge to the same output.

### 6.4 Journey docs

`docs/` contains human-authored Markdown files for the effort: specs, design drafts, investigation notes, migration plans, review notes, and other working documents. These files are edited directly. They are not generated projections and are not overwritten by `checkpoint` or `resume`.

Journey docs are intentionally stored outside linked repos until the developer decides they belong in a repository. This avoids root-level repo clutter from temporary specs and half-finished planning docs.

The v0 CLI only needs basic document helpers:

- `journey doc new <name>` creates `docs/<name>.md`.
- `journey doc list` lists files under `docs/`.
- `journey doc path <name>` prints the absolute path to a doc.

There is no `artifacts/` directory in v0. If a future workflow needs raw logs, screenshots, or bulky outputs, that should be introduced as a separate concept based on real usage.

## 7. Environment capture — dirty snapshots

This is the part that earns the tool its keep, so it's specified concretely.

A checkpoint must durably capture **tracked uncommitted** work without disturbing the working tree, polluting branches, or touching the user's normal stash list. The user-facing term is **dirty snapshot**, not "stash", because Journey is not using the normal stash stack as user-visible workflow.

```sh
# Produce a commit object capturing index + working tree, WITHOUT modifying
# the working tree or pushing onto the stash stack:
snap=$(git -C "$worktree" stash create)          # empty if clean

# Pin it so it survives gc, namespaced under the journey:
git -C "$worktree" update-ref "refs/journey/$id/$seq-$name" "$snap"
```

`git stash create` yields a content-addressed commit for tracked dirty state and returns immediately, leaving the working tree exactly as it was. It does not run `git stash push`, does not clean/reset files, and does not add an entry to the user's normal stash list. `update-ref` pins the object under `refs/journey/<id>/...` so it is recoverable and GC-safe but invisible to normal branch/log output. The event stores `head`, `branch`, and `dirty_snapshot_ref`.

On resume, the dirty snapshot is recoverable three ways: show it (`git diff <head> <dirty_snapshot_ref>`), apply it (`git stash apply <dirty_snapshot_ref>`), or just report its `--stat`. Clean repos store `dirty_snapshot_ref: null` — no snapshot, nothing wasted.

Untracked files are recorded but not snapshotted in v0:

```sh
git -C "$worktree" ls-files --others --exclude-standard
```

The checkpoint event stores the untracked file paths so resume can warn clearly: "3 untracked files were present at checkpoint time but were not snapshotted." Capturing untracked contents can come later behind an explicit option such as `checkpoint --include-untracked`.

What is **not** auto-captured (because it's not machine-derivable and guessing erodes trust): env vars, running ports, feature-flag values, half-applied migrations. These enter as explicit `note`/`command` events when they matter. Journey captures what git can verify and lets the human/agent assert the rest *visibly and timestamped*, rather than silently pretending to know it.

## 8. Commands

### Journey context resolution

Any command that requires an existing Journey resolves its context this way:

1. If an explicit id is provided, load that Journey from the global index. For most commands this is `--journey <id>`; for commands such as `resume` and `status`, a positional `<id>` is also valid.
2. Otherwise, walk upward from the current directory looking for `journey.yaml`.
3. If no Journey folder is found, fail and ask for an explicit Journey id.

Journey does not infer context from arbitrary linked repo worktrees in v0. To run `journey note`, `journey checkpoint`, or other context-dependent commands without an explicit id, run them from inside the Journey folder.

Two commands carry the product. The rest support them.

### `journey checkpoint [-m <msg>] [--journey <id>]` — the forcing function

The cheap, frequent, side-effect-y heartbeat:

1. For each linked repo: read branch / HEAD / upstream / ahead-behind / tracked dirty state / untracked files; create a dirty snapshot ref if tracked files are dirty.
2. Append one `checkpoint` event.
3. Regenerate projections.

Cheap enough to run constantly. Designed to be invoked *automatically* — by the agent harness at natural boundaries (before a context switch, after a decision, when a question resolves) and optionally by a git `post-commit` hook. The discipline is removed by making it automatic.

### `journey resume [<id>]` — resumption as an action

Resume **rebuilds your environment**; it is not a document you read and pray is current.

1. Re-attach worktrees: for each linked repo, ensure the worktree exists (`git worktree add` if missing) at the recorded branch; report if HEAD has moved since the last checkpoint.
2. Surface uncommitted state: print `git diff --stat` of each dirty snapshot ref; warn about untracked files recorded at checkpoint time; offer `--apply` to restore tracked dirty state into the worktree.
3. Print the derived `NOW`: current state, next actions (with age), open questions, recent decisions, last commands run.
4. Flag drift: warn on anything that changed underneath the journey (upstream moved, branch deleted, repo path gone).

### Supporting commands

```
journey new <title>                 # create; scaffold is lazy
journey link <repo-path>            # add a repo to the effort (by reference)
journey note "<text>"               # append a note event
journey decide "<text>" [--because] # append a decision event
journey ask "<q>" / journey resolve <qid> "<a>"
journey next "<a>" "<b>" ...        # set next actions
journey doc new <name>              # create docs/<name>.md
journey doc list                    # list human-authored Journey docs
journey doc path <name>             # print the path to a Journey doc
journey list [--status active]      # discovery (see §11)
journey status [<id>]               # one-screen summary without full resume
journey pause|archive|abandon <id>  # lifecycle
```

## 9. The Skill / forcing function

The companion Skill is where Journey lives or dies, so its job is narrow and mechanical, not aspirational. It instructs the agent to:

- **Checkpoint at boundaries** — before switching efforts, after a decision, when a test flips, when a question resolves. (`checkpoint` is cheap by design so this is painless.)
- **Never hand-maintain projections** — write events, never edit `NOW.md`.
- **Emit structured events for the things that matter** — `decision`/`question`/`next_actions` rather than burying them in prose, so they survive into the projection.
- **Resume before acting** — start a session with `journey resume` to rebuild state rather than re-deriving from the codebase.

The forcing function is structural, not behavioral: resume reads only from the log, so anything the agent fails to log is simply invisible later. The cost of skipping is paid by the skipper. That's the only kind of discipline that holds.

## 10. Staleness & trust

Trust decay is the primary failure mode, so it's defended explicitly:

- **Verifiable state is checked, not believed.** On resume, recorded branch/HEAD are diffed against live git. If they disagree, resume says so instead of asserting stale data.
- **Asserted state is timestamped and aged.** `next_actions` carries its own age. If the latest one is 6 days and 4 checkpoints old, `NOW.md` renders `next actions (last set 6d ago, 4 checkpoints stale)` — the human sees exactly how much to trust it.
- **Projections are disposable.** They're never the source of truth, so they can never silently rot; regenerating from the log always yields current truth.

## 11. Lifecycle & discovery

Without these, a flat pile of efforts becomes the junk drawer it was meant to replace.

- **Global index** (`~/.journey/index.yaml`): every journey with `id`, `title`, `status`, `updated`, repo names. `journey list` reads this — no scanning.
- **Status field**: `active` → `paused` → `archived` / `abandoned`. `list` defaults to `active`.
- **Archival**: `journey archive` flips status and (optionally) drops snapshot refs to reclaim space, keeping the log + projections as a readable record. The effort's history survives; the environment scaffolding doesn't have to.
- **Ceremony scales**: a 20-minute "Review PR-1234" is a journey with zero linked repos and a handful of log lines. No `worktrees/`, no decisions file, nothing it didn't earn.

## 12. Concurrency model

- **Source of truth is append-only.** Events are appended rather than rewritten, so the history is easy to inspect and replay.
- **Provenance + ordering on every event.** `seq` (monotonic) + `session` id, so projections are deterministic and per-agent contributions are attributable.
- **Low local concurrency assumption.** v0 does not optimize for many simultaneous writers. `seq` is assigned by reading the current tail and appending the next event. If real concurrent writes become common, add a per-Journey file lock around sequence assignment and append.
- **Projections are idempotent.** Regeneration is a pure replay, so concurrent regenerations converge rather than corrupt.

(Multi-machine sync — two laptops, one effort — is explicitly out of scope for v0; see open questions.)

## 13. What Journey leans on git for

To stay honest about scope, the redundant parts are delegated outright to the git CLI: branches and commits, dirty snapshots (`git stash create` objects pinned under `refs/journey/*`), historical recovery (reflog), multi-checkout (worktrees), and content addressing / GC safety (refs). Journey writes **none** of its own VCS. It adds only the effort index, the event log, Journey-local docs, the symlinked worktree index, and resume.

## 14. v0 scope (to keep this actionable)

Ship the spine, defer the rest.

**In:** Rust CLI; `new`, `link`, `checkpoint`, `resume`, `list`, `note`, `decide`, `ask/resolve`, `next`, `doc new/list/path`; `journal.jsonl` + `journey.yaml`; `NOW.md` projection; `docs/`; symlinked `worktrees/`; dirty snapshot refs for tracked changes; untracked-file warnings; global index; explicit Journey context resolution; single machine; manual checkpoint + optional `post-commit` hook.

**Deferred:** auto-checkpoint heuristics in the harness; `DECISIONS.md`/`QUESTIONS.md` projections (log-only at first); multi-machine sync; untracked-content snapshots; snapshot apply UX beyond `--apply`; web/TUI views; artifacts/raw-output management.

## 15. Open questions

1. **Multi-machine.** Is an effort ever resumed on a different machine? If yes, snapshot refs and absolute worktree paths don't travel — needs a sync/export story (bundle refs? push to a `journey/*` remote namespace?). v0 assumes one machine.
2. **Secrets in capture.** `command` events and diffs can capture tokens/keys. Need a redaction pass or an opt-out for `command` capture before this is safe to share.
3. **Snapshot lifetime.** When are `refs/journey/*` snapshots pruned? On archive? On a TTL? They're cheap but unbounded growth is real.
4. **NOW edits.** Banner-protecting `NOW.md` is fine for agents; humans will still edit it. Do we hard-block (read-only file) or detect-and-fold-into-log on next checkpoint?
5. **`command` capture source.** Cleanest if the agent harness emits these automatically; absent that, is a shell wrapper/hook worth it, or do commands stay manual?
6. **Effort boundaries.** When one investigation splits into two, is there a `journey fork`, or does the human just `new` + `link` the same repos?
