# AGENTS.md

This file gives coding agents the current project context for Journey.

## Product Shape

Journey is durable, file-based memory for engineering work, operated by agents on the user's behalf. The full philosophy is in `docs/philosophy.md`; read it before changing the product surface.

In short:

- Agents are the primary users. The CLI is the interface, and every capability must work non-interactively.
- Journey is memory, not a workspace. It does not host terminals, sessions, or agents.
- Journey is self-explaining. Agents learn what it is and when to use it from the tool itself.
- Journey is unopinionated about what goes inside a journey. It does not prescribe document structure, workflow, or where state is kept, and different journeys may be run in completely different ways.
- Journey provides the container, the links between containers, and the lifecycle. Nothing more.

## Important Invariants

- Do not reintroduce checkpoints, snapshots, git stash workflows, dirty-state capture, restore/apply commands, or internal version management. Git owns code.
- Journey does not generate content inside a journey beyond initial scaffolding, and it never overwrites user content. `docs/`, `README.md` and per-journey `AGENTS.md` belong to the user.
- Anything users or agents write inside a journey, including status notes, task files and handoff docs, is legitimate content. Journey neither manages nor forbids it.
- Journey does not track tasks and has no structured narrative commands (`ask`, `decide`, `next`, and so on).
- Worktrees are referenced, not owned. The `worktrees/` directory contains symlinks as a convenience view only.
- A canonical worktree path can be attached to only one active or paused Journey at a time.
- Journeys that are no longer open release worktree ownership. Active and paused Journeys may own worktrees.
- Lifecycle commands do not modify code state.
- A journey's folder path is stable. Do not move journey folders on status or kind changes; agent sessions and docs refer to journeys by path.
- Context resolution must work from a Journey folder or from inside an attached worktree.

## Data Model

Default state root is `~/.journey`; tests and agents can set `JOURNEY_HOME`.

```text
~/.journey/
|-- JOURNEY-AGENTS.md
|-- index.yaml
|-- worktree-index.yaml
`-- journeys/
    `-- <journey-id>/
        |-- journey.yaml
        |-- journal.jsonl
        |-- AGENTS.md
        |-- CLAUDE.md
        |-- README.md
        |-- docs/
        `-- worktrees/
```

Key files:

- `journey.yaml`: id, title, optional description, status, created timestamp, linked repos.
- `index.yaml`: global registry used by list and discovery.
- `worktree-index.yaml`: canonical worktree path to owning Journey id and repo name.
- `journal.jsonl`: operational events only (`link_repo`, `unlink_repo`, `status_change`).
- `JOURNEY-AGENTS.md`: shared agent guidance, owned by Journey.
- `AGENTS.md`: per-journey pointer to the shared guidance; user-owned after creation.
- `CLAUDE.md`: reference to `AGENTS.md`.
- `README.md`: optional user-owned Journey overview.

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

## Code Map

- `src/cli.rs`: clap command definitions.
- `src/app/mod.rs`: command dispatch and core command behavior.
- `src/app/worktree.rs`: linking, unlinking and removing worktrees.
- `src/storage.rs`: files, indexes, context resolution, symlinks, atomic writes, shared agent guidance.
- `src/models.rs`: serialized data structures.
- `src/events.rs`: journal read/write and timestamps.
- `src/git.rs`: git CLI wrapper and worktree creation.
- `src/tui/`: interactive terminal app (being removed in v0.3.0).
- `src/config/`: TUI action ordering and shortcuts (being removed in v0.3.0).
- `tests/cli_flow.rs`: end-to-end CLI behavior.

## Development Notes

- Prefer existing patterns and keep the surface small.
- Every new capability needs a non-interactive CLI path and output an agent can rely on.
- Be careful with dirty worktrees. Do not revert unrelated changes.
- Avoid destructive git commands unless explicitly requested.
- Use conventional commit messages for commits.
- If adding dependencies, justify why the standard library, `clap`, `serde` and the git CLI are not enough.

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

Always test against a temporary `JOURNEY_HOME` so the real user state is not modified.
