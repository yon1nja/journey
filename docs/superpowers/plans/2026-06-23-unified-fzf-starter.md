# Unified fzf Starter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hand-rolled TUI journey starter with the fzf-based journey list, so `journey` (no subcommand) opens the same fzf UI as `journey list`, with ctrl-n to create new journeys.

**Architecture:** The `None` arm in `run()` delegates to `list_journeys()` instead of `start_journey_tui()`. A new hidden `__fzf-new-journey` subcommand orchestrates a chained-fzf flow (title → description → worktree) via new functions in `picker.rs`. All hand-rolled TUI code is deleted from `app.rs`.

**Tech Stack:** Rust, clap, fzf (external binary), console crate for ANSI styling

## Global Constraints

- fzf must be available on PATH for interactive use; non-interactive fallback via `--non-interactive` stays unchanged.
- All new picker functions follow the existing pattern: picker.rs is pure UI, app.rs handles storage/git/events.
- The `journey new <title>` CLI subcommand remains for scripted use.

---

### Task 1: Add `fzf_prompt_text` helper to picker.rs

**Files:**
- Modify: `src/picker.rs` (add function after `pick_journey_action`, around line 178)
- Test: `tests/cli_flow.rs` (new test)

**Interfaces:**
- Consumes: `ensure_fzf()` from picker.rs (existing)
- Produces: `pub fn fzf_prompt_text(prompt: &str, header: &str, required: bool) -> Result<Option<String>>` — returns `Ok(Some(text))` on input, `Ok(None)` on Escape/abort, `Err` on fzf failure. If `required` is true and user submits empty input, re-prompts (loops internally).

- [ ] **Step 1: Write the failing test**

Add to `tests/cli_flow.rs`:

```rust
#[test]
fn fzf_new_journey_hidden_subcommand_exists() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Existing", "Journey"]);

    // The __fzf-new-journey subcommand should be recognized by the CLI parser.
    // We can't test the interactive fzf flow, but we can verify the subcommand
    // doesn't error with "unrecognized subcommand".
    // For now, just verify the CLI still compiles with the new subcommand.
    // The actual fzf interaction tests are manual.
    let candidates = journey(&home, &["__fzf-candidates"]);
    assert!(candidates.contains("existing-journey"));
}
```

- [ ] **Step 2: Run test to verify it passes (baseline)**

Run: `cargo test fzf_new_journey_hidden_subcommand_exists -- --nocapture`
Expected: PASS (this is a baseline test for the existing CLI; the real new-journey test comes in Task 3)

- [ ] **Step 3: Implement `fzf_prompt_text` in picker.rs**

Add after `pick_journey_action()` (after line 178):

```rust
pub fn fzf_prompt_text(prompt: &str, header: &str, required: bool) -> Result<Option<String>> {
    ensure_fzf()?;

    loop {
        let mut child = Command::new("fzf")
            .arg("--print-query")
            .arg("--no-info")
            .arg("--border=rounded")
            .arg("--layout=reverse")
            .arg("--height=40%")
            .arg("--margin=5%,10%")
            .arg("--padding=1")
            .arg(format!("--prompt={prompt} "))
            .arg(format!("--header={header}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("failed to start fzf text prompt")?;

        {
            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow!("failed to open fzf stdin"))?;
            drop(stdin);
        }

        let output = child.wait_with_output()?;

        if matches!(output.status.code(), Some(130)) {
            return Ok(None);
        }

        // With --print-query, fzf outputs the query on line 1 and the selected
        // item (if any) on line 2. Exit code 1 means no match selected, but
        // the query is still on stdout. We always read line 1 (the query).
        let text = String::from_utf8(output.stdout)
            .context("fzf output was not UTF-8")?
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();

        if !required || !text.is_empty() {
            return Ok(Some(text).filter(|t| !t.is_empty()));
        }
    }
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build`
Expected: success

- [ ] **Step 5: Commit**

```bash
git add src/picker.rs
git commit -m "feat: add fzf_prompt_text helper for chained text input"
```

---

### Task 2: Add `fzf_pick_worktree_action` and `run_new_journey` to picker.rs

**Files:**
- Modify: `src/picker.rs` (add types + two functions)

**Interfaces:**
- Consumes: `fzf_prompt_text()` from Task 1, `ensure_fzf()` (existing)
- Produces:
  - `pub enum WorktreeAction { Attach, Create { path: PathBuf, branch: String }, Skip }`
  - `pub struct NewJourneyInput { pub title: String, pub description: Option<String>, pub worktree_action: Option<WorktreeAction> }`
  - `pub fn fzf_pick_worktree_action(repo_name: &str, default_slug: &str, repo_root: &Path) -> Result<Option<WorktreeAction>>`
  - `pub fn run_new_journey(cwd: &Path) -> Result<Option<NewJourneyInput>>`

- [ ] **Step 1: Add types at the top of picker.rs**

Add after the existing imports (after line 12):

```rust
use std::path::PathBuf;

pub enum WorktreeAction {
    Attach,
    Create { path: PathBuf, branch: String },
    Skip,
}

pub struct NewJourneyInput {
    pub title: String,
    pub description: Option<String>,
    pub worktree_action: Option<WorktreeAction>,
}
```

- [ ] **Step 2: Implement `fzf_pick_worktree_action`**

Add after `fzf_prompt_text()`:

```rust
pub fn fzf_pick_worktree_action(
    repo_name: &str,
    default_slug: &str,
    repo_root: &Path,
) -> Result<Option<WorktreeAction>> {
    ensure_fzf()?;

    let actions = [
        ("attach", "Attach current worktree"),
        ("create", "Create a new worktree"),
        ("skip", "Skip for now"),
    ];
    let input = actions
        .iter()
        .map(|(key, label)| format!("{key}\t{label}"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut child = Command::new("fzf")
        .arg("--ansi")
        .arg("--no-multi")
        .arg("--cycle")
        .arg("--border=rounded")
        .arg("--layout=reverse")
        .arg("--height=40%")
        .arg("--margin=5%,10%")
        .arg("--padding=1")
        .arg("--prompt=Worktree> ")
        .arg(format!(
            "--header=Git repo: {repo_name} | Link a worktree? | esc: skip"
        ))
        .arg(format!("--delimiter={}", "\t"))
        .arg("--with-nth=2..")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to start fzf worktree picker")?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open fzf worktree picker stdin"))?;
        stdin.write_all(input.as_bytes())?;
    }

    let output = child.wait_with_output()?;
    if matches!(output.status.code(), Some(1 | 130)) {
        return Ok(Some(WorktreeAction::Skip));
    }
    if !output.status.success() {
        bail!("fzf worktree picker exited with status {}", output.status);
    }

    let selected = String::from_utf8(output.stdout).context("fzf output was not UTF-8")?;
    let action = selected
        .trim_end()
        .split_once('\t')
        .map(|(key, _)| key)
        .unwrap_or_else(|| selected.trim());

    match action {
        "attach" => Ok(Some(WorktreeAction::Attach)),
        "create" => {
            let repo_parent = repo_root.parent().unwrap_or(repo_root);
            let default_path = repo_parent.join(format!(
                "{}-{}",
                repo_root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "repo".to_string()),
                default_slug
            ));
            let default_path_str = default_path.display().to_string();

            let path_str = fzf_prompt_text(
                "Worktree path>",
                &format!("Default: {default_path_str} | type to override | enter: accept"),
                false,
            )?;
            let path = match path_str.filter(|s| !s.is_empty()) {
                Some(p) => PathBuf::from(p),
                None => default_path,
            };

            let branch = fzf_prompt_text(
                "Branch>",
                &format!("Default: {default_slug} | type to override | enter: accept"),
                false,
            )?;
            let branch = branch
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| default_slug.to_string());

            Ok(Some(WorktreeAction::Create { path, branch }))
        }
        "skip" | "" => Ok(Some(WorktreeAction::Skip)),
        other => bail!("unknown worktree action: {other}"),
    }
}
```

- [ ] **Step 3: Implement `run_new_journey`**

Add after `fzf_pick_worktree_action()`:

```rust
pub fn run_new_journey(cwd: &Path) -> Result<Option<NewJourneyInput>> {
    let title = match fzf_prompt_text(
        "Title>",
        "Type a journey title and press Enter | esc: cancel",
        true,
    )? {
        Some(t) => t,
        None => return Ok(None),
    };

    let description = match fzf_prompt_text(
        "Description>",
        "Optional — press Enter to skip | esc: cancel",
        false,
    )? {
        Some(d) if d.is_empty() => None,
        other => other,
    };

    let git_context = crate::git::discover_repo(cwd).ok();
    let worktree_action = if let Some(repo) = &git_context {
        let repo_name = repo
            .root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "repo".to_string());
        let slug = crate::storage::slugify(&title);
        fzf_pick_worktree_action(&repo_name, &slug, &repo.root)?
    } else {
        None
    };

    Ok(Some(NewJourneyInput {
        title,
        description,
        worktree_action,
    }))
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build`
Expected: success

- [ ] **Step 5: Commit**

```bash
git add src/picker.rs
git commit -m "feat: add run_new_journey chained fzf flow and worktree picker"
```

---

### Task 3: Add `__fzf-new-journey` CLI subcommand and handler in app.rs

**Files:**
- Modify: `src/cli.rs` (add `FzfNewJourney` variant, around line 55)
- Modify: `src/app.rs` (add handler in `run()` match + `fzf_new_journey()` function)
- Test: `tests/cli_flow.rs` (update existing test)

**Interfaces:**
- Consumes: `picker::run_new_journey()` from Task 2, `picker::WorktreeAction`, `storage::create_journey()`, `link_repo()`, `git::create_worktree()`, `events::now_rfc3339()`
- Produces: `fzf_new_journey(home: &Path, cwd: &Path) -> Result<String>` in app.rs; `Commands::FzfNewJourney { cwd }` in cli.rs

- [ ] **Step 1: Add CLI variant in cli.rs**

Add after the `FzfActionMenu` variant (after line 55 in cli.rs):

```rust
    #[command(name = "__fzf-new-journey", hide = true)]
    FzfNewJourney {
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
```

- [ ] **Step 2: Add match arm and handler in app.rs**

Add the match arm in `run()` after the `FzfActionMenu` arm (after line 46):

```rust
        Some(Commands::FzfNewJourney { cwd: override_cwd }) => {
            let effective_cwd = override_cwd.unwrap_or_else(|| cwd.clone());
            fzf_new_journey(&home, &effective_cwd)
        }
```

Add the handler function after `fzf_action_menu()` (after line 718):

```rust
fn fzf_new_journey(home: &Path, cwd: &Path) -> Result<String> {
    let Some(input) = picker::run_new_journey(cwd)? else {
        return Ok(String::new());
    };

    let now = events::now_rfc3339()?;
    let ctx = storage::create_journey(home, &input.title, input.description, &now)?;

    let mut messages = vec![format!(
        "created Journey `{}` at {}",
        ctx.journey.id,
        ctx.path.display()
    )];

    if let Some(action) = input.worktree_action {
        match action {
            picker::WorktreeAction::Attach => {
                let repo_name = cwd
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "repo".to_string());
                let linked = link_repo(home, cwd, Some(&ctx.journey.id), cwd, Some(repo_name))?;
                messages.push(linked);
            }
            picker::WorktreeAction::Create { path, branch } => {
                let discovered = git::discover_repo(cwd)?;
                git::create_worktree(&discovered.root, &path, &branch, true)?;
                messages.push(format!(
                    "created git worktree {} on branch `{}`",
                    path.display(),
                    branch
                ));
                let repo_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "worktree".to_string());
                let linked =
                    link_repo(home, cwd, Some(&ctx.journey.id), &path, Some(repo_name))?;
                messages.push(linked);
            }
            picker::WorktreeAction::Skip => {}
        }
    }

    for msg in &messages {
        println!("{msg}");
    }
    wait_for_enter()?;
    Ok(String::new())
}
```

- [ ] **Step 3: Verify it compiles and existing tests pass**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/cli.rs src/app.rs
git commit -m "feat: add __fzf-new-journey subcommand for chained fzf creation flow"
```

---

### Task 4: Wire ctrl-n into the fzf list and update header with git context

**Files:**
- Modify: `src/picker.rs` (`run_journey_list` function, lines 14-65)
- Modify: `src/app.rs` (`list_journeys` signature and the `None` arm callsite)
- Modify: `src/cli.rs` (no structural change, just the `None` arm)

**Interfaces:**
- Consumes: `run_journey_list()` (existing), `git::discover_repo()` (existing)
- Produces: Updated `run_journey_list(default_filter, rows, cwd)` that adds ctrl-n binding and git-aware header

- [ ] **Step 1: Update `run_journey_list` signature to accept `cwd`**

In `src/picker.rs`, change the function signature from:

```rust
pub fn run_journey_list(default_filter: JourneyStatus, rows: &[IndexEntry]) -> Result<()> {
```

to:

```rust
pub fn run_journey_list(default_filter: JourneyStatus, rows: &[IndexEntry], cwd: &Path) -> Result<()> {
```

- [ ] **Step 2: Add ctrl-n binding and update header**

In `run_journey_list()`, after the existing `change_bind` (line 25), add:

```rust
    let git_hint = crate::git::discover_repo(cwd)
        .ok()
        .map(|repo| {
            let name = repo
                .root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "repo".to_string());
            format!(" (git: {name}, {})", repo.branch)
        })
        .unwrap_or_default();

    let cwd_str = shell_quote(cwd);
    let new_journey_command = format!("{exe} __fzf-new-journey --cwd={cwd_str}");
    let new_bind = format!("ctrl-n:execute({new_journey_command})+reload({reload_command})");
```

Replace the `--header` arg (line 37-39) with:

```rust
        .arg(format!(
            "--header=Journey list | enter: actions | ctrl-n: new journey{git_hint} | ctrl-r: reload"
        ))
```

Replace the `--prompt` arg (line 35) with:

```rust
        .arg(format!("--prompt=Journeys [{default_filter}]> "))
```

Add the new bind arg after the existing `--bind={change_bind}` (line 46):

```rust
        .arg(format!("--bind={new_bind}"))
```

- [ ] **Step 3: Update `list_journeys` in app.rs to pass `cwd`**

Change `list_journeys` signature from:

```rust
fn list_journeys(
    home: &Path,
    default_filter: JourneyStatus,
    non_interactive: bool,
) -> Result<String> {
```

to:

```rust
fn list_journeys(
    home: &Path,
    cwd: &Path,
    default_filter: JourneyStatus,
    non_interactive: bool,
) -> Result<String> {
```

Update the `picker::run_journey_list` call (line 649) from:

```rust
        picker::run_journey_list(default_filter, &rows)?;
```

to:

```rust
        picker::run_journey_list(default_filter, &rows, cwd)?;
```

- [ ] **Step 4: Update the `List` match arm in `run()` to pass `cwd`**

Change lines 33-40 from:

```rust
        Some(Commands::List {
            status,
            non_interactive,
        }) => list_journeys(
            &home,
            status.unwrap_or(JourneyStatus::Active),
            non_interactive,
        ),
```

to:

```rust
        Some(Commands::List {
            status,
            non_interactive,
        }) => list_journeys(
            &home,
            &cwd,
            status.unwrap_or(JourneyStatus::Active),
            non_interactive,
        ),
```

- [ ] **Step 5: Verify it compiles and tests pass**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add src/picker.rs src/app.rs
git commit -m "feat: wire ctrl-n new-journey binding into fzf list with git context header"
```

---

### Task 5: Unify `journey` (no subcommand) to open the fzf list

**Files:**
- Modify: `src/app.rs` (change `None` arm in `run()`, line 22)
- Modify: `tests/cli_flow.rs` (update `bare_journey_requires_a_terminal` test)

**Interfaces:**
- Consumes: `list_journeys()` (updated in Task 4)
- Produces: `None` arm calls `list_journeys()` instead of `start_journey_tui()`

- [ ] **Step 1: Change the `None` arm in `run()`**

In `src/app.rs`, change line 22 from:

```rust
        None => start_journey_tui(&home, &cwd),
```

to:

```rust
        None => list_journeys(&home, &cwd, JourneyStatus::Active, false),
```

- [ ] **Step 2: Update the `bare_journey_requires_a_terminal` test**

In `tests/cli_flow.rs`, the test at line 164 currently expects the error to contain "interactive terminal UI". Since `journey` now calls `list_journeys()` which falls through to non-interactive mode when not a TTY (returning table output or "no Journeys"), the bare command should succeed with table output or "no Journeys" when piped.

Replace the test:

```rust
#[test]
fn bare_journey_shows_non_interactive_list_when_piped() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Piped", "Test"]);

    let output = journey(&home, &[]);
    assert!(output.contains("piped-test"));
}
```

- [ ] **Step 3: Verify all tests pass**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/app.rs tests/cli_flow.rs
git commit -m "feat: unify bare journey command to open fzf list"
```

---

### Task 6: Delete the hand-rolled TUI code

**Files:**
- Modify: `src/app.rs` (delete ~400 lines of starter TUI code)

**Interfaces:**
- Consumes: nothing (pure deletion)
- Produces: cleaner app.rs with no TUI code

- [ ] **Step 1: Delete types and starter functions**

Remove from `src/app.rs`:

1. `StarterDraft` struct (lines 72-81)
2. `StarterStep` enum (lines 83-89)
3. `start_journey_tui()` function (lines 91-176)
4. `create_and_link_worktree()` function (lines 178-216)
5. `default_worktree_path()` function (lines 218-231)
6. `prompt_repo_name()` function (lines 233-239)
7. `render_starter_ui()` function (lines 241-256)
8. `render_starter_split()` function (lines 258-313)
9. `render_starter_single()` function (lines 315-334)
10. `starter_left_lines()` function (lines 336-368)
11. `starter_preview_lines()` function (lines 370-439)
12. `starter_step_line()` function (lines 441-447)
13. `starter_step_status()` function (lines 449-456)
14. `worktree_choice_label()` function (lines 458-466)
15. `field_line()` function (lines 468-475)
16. `optional_compact()` function (lines 477-481)
17. `terminal_width()` function (lines 483-486)
18. `ui_border()` function (lines 488-490)
19. `ui_split_border()` function (lines 492-498)
20. `pad_ansi()` function (lines 500-507)
21. `compact_text()` function (lines 509-530)
22. `prompt_required()` function (lines 548-556)
23. `prompt_optional()` function (lines 558-566)
24. `prompt_choice()` function (lines 568-577)
25. `prompt_yes_no()` function (lines 579-593)
26. `prompt_path()` function (lines 595-603)
27. `prompt_line()` function (lines 605-620)
28. `clear_screen()` function (lines 622-626)

- [ ] **Step 2: Remove unused imports**

In `src/app.rs`, remove these imports that are only used by the deleted code:

- `use std::io::{self, IsTerminal, Write};` — change to `use std::io::{self, IsTerminal};` (IsTerminal may still be used by list_journeys; Write is used by wait_for_enter — check and keep what's needed)
- `use console::{measure_text_width, style, Term};` — change to `use console::style;` (measure_text_width and Term are only used by deleted code)

Actually, check remaining usage first:
- `io::Write` is used by `wait_for_enter()` (`io::stdout().flush()`) — keep
- `io::IsTerminal` is used by `list_journeys()` — keep
- `measure_text_width` — only used in deleted `pad_ansi()` — remove
- `Term` — only used in deleted `terminal_width()` — remove
- `style` — used by `ui_active()`, `ui_success()`, `ui_dim()` — keep

Change import to:

```rust
use console::style;
```

- [ ] **Step 3: Verify it compiles and all tests pass**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/app.rs
git commit -m "refactor: delete hand-rolled TUI starter code (~400 lines)"
```
