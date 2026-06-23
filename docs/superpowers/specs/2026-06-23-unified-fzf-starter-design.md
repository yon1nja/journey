# Unified fzf Starter Design

## Problem

The `journey` (no-subcommand) interactive starter uses a hand-rolled TUI with box-drawing, split panes, and custom prompt functions. It looks and feels completely different from `journey list`, which uses fzf for a polished, responsive experience. The goal is to unify both into a single fzf-based interface.

## Design

### Unified entry point

`journey` (no subcommand) becomes equivalent to `journey list`. The `start_journey_tui()` function and all supporting TUI code (~350 lines) are deleted. The `None` arm in `run()` calls `list_journeys()` instead.

`journey new <title>` stays for non-interactive/scripted use.

### New journey creation via ctrl-n

A `ctrl-n` keybinding is added to `run_journey_list()`. It triggers `execute({exe} __fzf-new-journey --cwd={cwd})` followed by `reload(...)` so the newly created journey appears in the list.

The hidden `__fzf-new-journey` subcommand runs a chained fzf flow:

1. **Title step**: fzf with `--print-query`, empty candidate list, `--prompt=Title> `, `--header=Type a journey title and press Enter`. The query is captured as the title. Required — empty aborts.

2. **Description step**: Same `--print-query` pattern, `--prompt=Description> `, `--header=Optional — press Enter to skip`. Empty input is accepted.

3. **Worktree step** (only if git repo detected in cwd): fzf picker with candidates:
   - `attach` — Attach current worktree
   - `create` — Create a new worktree
   - `skip` — Skip for now

   If "create" is selected, follow-up fzf prompts for worktree path and branch name (same `--print-query` pattern with sensible defaults shown in the header).

4. Journey is created, summary printed, control returns to the main list via reload.

Escape at any step aborts the entire chain (no journey created).

### Smart header with git context

`run_journey_list()` accepts an optional `cwd` parameter. When inside a git repo, the fzf header shows:

```
Journey list | enter: actions | ctrl-n: new journey (git: repo-name, branch)
```

When not in a git repo:

```
Journey list | enter: actions | ctrl-n: new journey
```

The current status filter info moves from header to prompt: `Journeys [active]> `.

### New code in picker.rs

- `pub fn run_new_journey(cwd: &Path) -> Result<Option<NewJourneyInput>>` — orchestrates the chained fzf flow.
- `NewJourneyInput` struct: `title: String`, `description: Option<String>`, `worktree_action: WorktreeAction`.
- `WorktreeAction` enum: `Attach`, `Create { path: PathBuf, branch: String }`, `Skip`.
- `fzf_prompt_text(prompt, header, required) -> Result<Option<String>>` — reusable fzf `--print-query` wrapper. Returns `None` on Escape.
- `fzf_pick_worktree_action(repo_hint) -> Result<WorktreeAction>` — picker using existing action menu pattern.

Picker stays pure UI. Actual journey creation (storage, git, events) happens in `app.rs` in the `__fzf-new-journey` handler.

### CLI changes

- Add `FzfNewJourney` hidden variant to `Commands` with optional `--cwd <path>` (defaults to current dir).
- `None` arm in `run()` calls `list_journeys()` with cwd for git context.
- `list_journeys()` signature adds `cwd: &Path` parameter.

### Code deletion from app.rs

Removed:
- `StarterDraft`, `StarterStep`
- `start_journey_tui()`, `create_and_link_worktree()`, `default_worktree_path()`
- `render_starter_ui()`, `render_starter_split()`, `render_starter_single()`
- `starter_left_lines()`, `starter_preview_lines()`, `starter_step_line()`, `starter_step_status()`, `worktree_choice_label()`
- `field_line()`, `optional_compact()`
- `terminal_width()`, `ui_border()`, `ui_split_border()`, `pad_ansi()`, `compact_text()`
- `prompt_required()`, `prompt_optional()`, `prompt_choice()`, `prompt_yes_no()`, `prompt_path()`, `prompt_line()`, `prompt_repo_name()`, `clear_screen()`

Kept: `ui_active()`, `ui_success()`, `ui_dim()`, `ui_label()` — used by `status()` and other non-TUI code paths.
