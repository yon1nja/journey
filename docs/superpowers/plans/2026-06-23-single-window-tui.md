# Single-Window TUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the multi-fzf-window TUI with a single fzf instance that uses `transform`/`reload`/`change-prompt`/`change-header` to switch between modes (journey list, action menu, wizard steps, pickers) in-place, keeping the preview pane always visible.

**Architecture:** A single fzf process runs for the entire session. Mode is encoded in item prefixes (`act:`, `wiz:`, `repo:`, `new:`). A new `__fzf-transform` CLI subcommand acts as the brain — fzf calls it on Enter/Esc and it returns fzf action strings that reload items, change prompts, execute actions, etc. Simple actions run via `execute-silent` + header notification. Complex flows (worktree wizard, new journey) use `reload` to transition between wizard steps within the same fzf.

**Tech Stack:** Rust, fzf (requires 0.41+ for `transform` bind), clap

## Global Constraints

- fzf 0.41+ required (for `transform` action in `--bind`)
- All item keys use `\t` as field delimiter; first field encodes mode+state
- Preview always shows journey details; for new journeys it builds up progressively
- `shell` action still needs `execute()` (takes over terminal for interactive shell)
- The `change` bind for live search only applies in list mode

---

### Task 1: Add `__fzf-transform`, `__fzf-action-items`, and `__fzf-dispatch` CLI subcommands

**Files:**
- Modify: `src/cli.rs:18-80` (add new hidden subcommands to `Commands` enum)
- Modify: `src/app.rs:16-64` (add dispatch arms in `run()`)

**Interfaces:**
- Produces: `Commands::FzfTransform { event, item, query, cwd }`, `Commands::FzfActionItems { id }`, `Commands::FzfDispatch { id, action, query, cwd }`
- Consumed by: Task 2 (fzf binds call these subcommands)

- [ ] **Step 1: Add CLI variants**

In `src/cli.rs`, add three new hidden subcommands to the `Commands` enum:

```rust
    #[command(name = "__fzf-action-items", hide = true)]
    FzfActionItems { id: String },
    #[command(name = "__fzf-dispatch", hide = true)]
    FzfDispatch {
        id: String,
        action: String,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    #[command(name = "__fzf-transform", hide = true)]
    FzfTransform {
        event: String,
        item: String,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
```

- [ ] **Step 2: Add `fzf_action_items()` in app.rs**

This returns action items with `act:{journey_id}:{key}` prefix for a given journey:

```rust
fn fzf_action_items(journey_id: &str) -> Result<String> {
    const JOURNEY_ACTIONS: [(&str, &str); 10] = [
        ("shell", "cd journey"),
        ("resume", "Resume"),
        ("worktree", "New branch + worktree"),
        ("link", "Link current worktree"),
        ("unlink", "Unlink a repo"),
        ("status", "Status"),
        ("path", "Print Journey path"),
        ("pause", "Pause"),
        ("archive", "Archive"),
        ("abandon", "Abandon"),
    ];
    let lines: Vec<String> = JOURNEY_ACTIONS
        .iter()
        .map(|(key, label)| format!("act:{journey_id}:{key}\t{label}"))
        .collect();
    Ok(lines.join("\n"))
}
```

- [ ] **Step 3: Add `fzf_dispatch()` in app.rs**

This executes a single action and returns a result message (printed to stdout). It handles all the "simple" actions that don't need further user input:

```rust
fn fzf_dispatch(home: &Path, cwd: &Path, journey_id: &str, action: &str, query: Option<&str>) -> Result<String> {
    match action {
        "resume" => resume(home, cwd, Some(journey_id)),
        "link" => link_repo(home, cwd, Some(journey_id), cwd, None),
        "status" => status(home, cwd, Some(journey_id)),
        "path" => Ok(storage::journey_dir(home, journey_id).display().to_string()),
        "pause" => set_status(home, cwd, Some(journey_id), JourneyStatus::Paused),
        "archive" => set_status(home, cwd, Some(journey_id), JourneyStatus::Archived),
        "abandon" => set_status(home, cwd, Some(journey_id), JourneyStatus::Abandoned),
        "unlink" => {
            let repo_name = query.unwrap_or("");
            if repo_name.is_empty() {
                bail!("no repo selected");
            }
            unlink_repo(home, cwd, Some(journey_id), repo_name)
        }
        "worktree" => {
            // query format: "branch\tpath"
            let q = query.unwrap_or("");
            let (branch, path) = q.split_once('\t').unwrap_or((q, ""));
            if branch.is_empty() || path.is_empty() {
                bail!("missing branch or path for worktree creation");
            }
            let discovered = git::discover_repo(cwd)?;
            let wt_path = PathBuf::from(path);
            git::create_worktree(&discovered.root, &wt_path, branch, true)?;
            let repo_name = wt_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "worktree".to_string());
            let linked = link_repo(home, cwd, Some(journey_id), &wt_path, Some(repo_name))?;
            Ok(format!(
                "created worktree {} on branch `{}`\n{}",
                wt_path.display(),
                branch,
                linked
            ))
        }
        "new-journey" => {
            // query format: "title\tdescription"
            let q = query.unwrap_or("");
            let (title, desc) = q.split_once('\t').unwrap_or((q, ""));
            if title.is_empty() {
                bail!("title is required");
            }
            let desc = if desc.is_empty() { None } else { Some(desc.to_string()) };
            new_journey(home, title, desc)
        }
        "new-journey-attach" => {
            let journey_id_actual = query.unwrap_or("");
            if journey_id_actual.is_empty() {
                bail!("missing journey id");
            }
            link_repo(home, cwd, Some(journey_id_actual), cwd, None)
        }
        "new-journey-worktree" => {
            // query format: "journey_id\tbranch\tpath"
            let q = query.unwrap_or("");
            let parts: Vec<&str> = q.splitn(3, '\t').collect();
            if parts.len() < 3 {
                bail!("missing journey_id, branch, or path");
            }
            let (jid, branch, path) = (parts[0], parts[1], parts[2]);
            let discovered = git::discover_repo(cwd)?;
            let wt_path = PathBuf::from(path);
            git::create_worktree(&discovered.root, &wt_path, branch, true)?;
            let repo_name = wt_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "worktree".to_string());
            let linked = link_repo(home, cwd, Some(jid), &wt_path, Some(repo_name))?;
            Ok(format!(
                "created worktree {} on branch `{}`\n{}",
                wt_path.display(),
                branch,
                linked
            ))
        }
        _ => bail!("unknown dispatch action: {action}"),
    }
}
```

- [ ] **Step 4: Add `fzf_transform()` in app.rs**

This is the brain. It receives an event name, the selected item key, and the current query, and returns a string of fzf actions. It needs access to the exe path and cwd for constructing commands.

```rust
use std::path::PathBuf;

fn fzf_transform(home: &Path, cwd: &Path, event: &str, item: &str, query: Option<&str>) -> Result<String> {
    let exe = shell_quote(&env::current_exe().context("failed to resolve current executable")?);
    let cwd_str = shell_quote(cwd);
    let reload_list = format!("{exe} __fzf-candidates --query={{q}}");

    match event {
        "enter" => fzf_transform_enter(home, cwd, &exe, &cwd_str, &reload_list, item, query),
        "esc" => fzf_transform_esc(&exe, &reload_list, item),
        _ => Ok(String::new()),
    }
}

fn shell_quote(path: &Path) -> String {
    let value = path.display().to_string();
    format!("'{}'", value.replace('\'', "'\\''"))
}
```

The `fzf_transform_enter` and `fzf_transform_esc` functions are defined in Task 2 and Task 3.

- [ ] **Step 5: Wire up dispatch in `run()`**

In `src/app.rs`, add match arms for the three new commands inside the `run()` function:

```rust
        Some(Commands::FzfActionItems { id }) => fzf_action_items(&id),
        Some(Commands::FzfDispatch { id, action, query, cwd: override_cwd }) => {
            let effective_cwd = override_cwd.unwrap_or_else(|| cwd.clone());
            fzf_dispatch(&home, &effective_cwd, &id, &action, query.as_deref())
        }
        Some(Commands::FzfTransform { event, item, query, cwd: override_cwd }) => {
            let effective_cwd = override_cwd.unwrap_or_else(|| cwd.clone());
            fzf_transform(&home, &effective_cwd, &event, &item, query.as_deref())
        }
```

- [ ] **Step 6: Build and verify compilation**

Run: `cargo build 2>&1`
Expected: successful build, no errors

- [ ] **Step 7: Commit**

```bash
git add src/cli.rs src/app.rs
git commit -m "feat: add fzf-transform, fzf-action-items, fzf-dispatch subcommands

Backbone for single-window TUI: transform returns fzf action strings,
action-items emits prefixed menu entries, dispatch executes actions."
```

---

### Task 2: Implement transform logic for Enter (list → actions, action dispatch, wizard steps)

**Files:**
- Modify: `src/app.rs` (add `fzf_transform_enter()` and wizard-step helpers)

**Interfaces:**
- Consumes: `fzf_transform()` from Task 1, `fzf_dispatch()` from Task 1
- Produces: `fzf_transform_enter()` — returns fzf action strings for each mode/item prefix

The transform handler inspects the item prefix and returns the right fzf actions:

- **No prefix** (journey ID) → switch to action menu via reload
- **`act:{id}:{action}`** → dispatch action or start wizard
- **`repo:{id}:{repo_name}`** → unlink the repo
- **`wiz:{id}:branch:{default}`** → capture branch from query, reload to path step
- **`wiz:{id}:path:{branch}:{default}`** → capture path from query, execute worktree creation
- **`new:title`** → capture title from query, reload to description step
- **`new:desc:{title}`** → capture description, create journey, reload to worktree step or list
- **`new:wt:{jid}:{action}`** → handle new journey worktree action (attach/create/skip)
- **`new:wt-branch:{jid}:{default}`** → capture branch, reload to path step
- **`new:wt-path:{jid}:{branch}:{default}`** → capture path, execute, return to list

- [ ] **Step 1: Implement `fzf_transform_enter()`**

Add to `src/app.rs`:

```rust
fn fzf_transform_enter(
    home: &Path,
    cwd: &Path,
    exe: &str,
    cwd_str: &str,
    reload_list: &str,
    item: &str,
    query: Option<&str>,
) -> Result<String> {
    let q = query.unwrap_or("");

    // --- Journey list mode: switch to action menu ---
    if !item.contains(':') && !item.is_empty() {
        let journey_id = item;
        let reload_actions = format!("{exe} __fzf-action-items {journey_id}");
        let preview = format!("{exe} __fzf-preview {journey_id}");
        return Ok(format!(
            "reload({reload_actions})+change-prompt(Action> )+change-header(Actions for {journey_id} | esc: back)+change-preview({preview})+clear-query"
        ));
    }

    // --- Action menu mode ---
    if let Some(rest) = item.strip_prefix("act:") {
        let (journey_id, action) = rest.split_once(':').unwrap_or((rest, ""));
        return fzf_transform_action(home, cwd, exe, cwd_str, reload_list, journey_id, action);
    }

    // --- Unlink picker mode ---
    if let Some(rest) = item.strip_prefix("repo:") {
        let (journey_id, repo_name) = rest.split_once(':').unwrap_or((rest, ""));
        let dispatch = format!(
            "{exe} __fzf-dispatch {journey_id} unlink --query={} --cwd={cwd_str}",
            shell_quote_value(repo_name)
        );
        return Ok(format!(
            "execute-silent({dispatch})+reload({reload_list})+change-prompt(Journeys> )+transform-header({exe} __fzf-dispatch {journey_id} unlink --query={} --cwd={cwd_str} 2>&1 || true)+clear-query+enable-search",
            shell_quote_value(repo_name)
        ));
    }

    // --- Worktree wizard: branch step ---
    if let Some(rest) = item.strip_prefix("wiz:") {
        return fzf_transform_wizard(home, cwd, exe, cwd_str, reload_list, rest, q);
    }

    // --- New journey wizard ---
    if let Some(rest) = item.strip_prefix("new:") {
        return fzf_transform_new_journey(home, cwd, exe, cwd_str, reload_list, rest, q);
    }

    Ok(String::new())
}
```

- [ ] **Step 2: Implement `fzf_transform_action()` — routes action menu Enter**

```rust
fn fzf_transform_action(
    home: &Path,
    cwd: &Path,
    exe: &str,
    cwd_str: &str,
    reload_list: &str,
    journey_id: &str,
    action: &str,
) -> Result<String> {
    match action {
        // Shell needs execute (interactive terminal)
        "shell" => {
            let dir = storage::journey_dir(home, journey_id);
            let dir_str = shell_quote(&dir);
            let shell_cmd = format!("$SHELL -l");
            Ok(format!(
                "execute(cd {dir_str} && {shell_cmd})+reload({reload_list})+change-prompt(Journeys> )+change-header(Journey list | enter: actions | ctrl-n: new journey | ctrl-r: reload)+clear-query+enable-search"
            ))
        }

        // Worktree wizard: switch to branch input step
        "worktree" => {
            let slug = storage::slugify(journey_id);
            let discovered = git::discover_repo(cwd)?;
            let default_path = discovered.root.join(format!(".worktrees/{slug}"));
            let default_path_str = default_path.display().to_string();
            let item_key = format!("wiz:{journey_id}:branch:{slug}");
            Ok(format!(
                "reload(echo '{item_key}\tDefault: {slug}')+change-prompt(Branch> )+change-header(New branch (default: {slug}) | type name + enter | esc: back)+clear-query+disable-search"
            ))
        }

        // Unlink: load repo list
        "unlink" => {
            let ctx = storage::resolve_context(home, Some(journey_id), cwd)?;
            if ctx.journey.repos.is_empty() {
                return Ok(format!(
                    "change-header(No repos linked to this journey | press any key)+change-prompt(> )"
                ));
            }
            let items: Vec<String> = ctx.journey.repos.iter().map(|r| {
                format!("repo:{journey_id}:{}\t{}  ({})", r.name, r.name, r.worktree.display())
            }).collect();
            let items_str = items.join("\n");
            let preview = format!("{exe} __fzf-preview {journey_id}");
            Ok(format!(
                "reload(printf '%s\\n' {})+change-prompt(Unlink> )+change-header(Select repo to unlink from {journey_id} | esc: back)+change-preview({preview})+clear-query+disable-search",
                shell_quote_value(&items_str)
            ))
        }

        // All other simple actions: dispatch silently, show result in header
        _ => {
            let dispatch = format!(
                "{exe} __fzf-dispatch {journey_id} {action} --cwd={cwd_str} 2>&1 || true"
            );
            let preview = format!("{exe} __fzf-preview {{1}}");
            Ok(format!(
                "execute-silent({exe} __fzf-dispatch {journey_id} {action} --cwd={cwd_str} 2>/dev/null)+reload({reload_list})+change-prompt(Journeys> )+transform-header({dispatch})+change-preview({preview})+clear-query+enable-search"
            ))
        }
    }
}
```

- [ ] **Step 3: Implement `fzf_transform_wizard()` — worktree wizard steps**

```rust
fn fzf_transform_wizard(
    _home: &Path,
    cwd: &Path,
    exe: &str,
    cwd_str: &str,
    reload_list: &str,
    rest: &str,
    query: &str,
) -> Result<String> {
    // rest = "{journey_id}:branch:{default}" or "{journey_id}:path:{branch}:{default_path}"
    let parts: Vec<&str> = rest.splitn(4, ':').collect();
    let journey_id = parts.first().unwrap_or(&"");

    match parts.get(1).copied() {
        Some("branch") => {
            let default_branch = parts.get(2).unwrap_or(&"");
            let branch = if query.is_empty() { default_branch } else { &query };
            let discovered = git::discover_repo(cwd)?;
            let default_path = discovered.root.join(format!(".worktrees/{branch}"));
            let default_path_str = default_path.display().to_string();
            let escaped_path = default_path_str.replace('\'', "'\\''");
            let item_key = format!("wiz:{journey_id}:path:{branch}:{escaped_path}");
            Ok(format!(
                "reload(echo '{item_key}\tDefault: {escaped_path}')+change-prompt(Path> )+change-header(Worktree path (default: {escaped_path}) | type path + enter | esc: back)+clear-query"
            ))
        }
        Some("path") => {
            let branch = parts.get(2).unwrap_or(&"");
            let default_path = parts.get(3).unwrap_or(&"");
            let path = if query.is_empty() { default_path } else { &query };
            let dispatch_query = format!("{branch}\t{path}");
            let dispatch = format!(
                "{exe} __fzf-dispatch {journey_id} worktree --query={} --cwd={cwd_str}",
                shell_quote_value(&dispatch_query)
            );
            let dispatch_msg = format!("{dispatch} 2>&1 || true");
            let preview = format!("{exe} __fzf-preview {{1}}");
            Ok(format!(
                "execute-silent({dispatch} 2>/dev/null)+reload({reload_list})+change-prompt(Journeys> )+transform-header({dispatch_msg})+change-preview({preview})+clear-query+enable-search"
            ))
        }
        _ => Ok(String::new()),
    }
}
```

- [ ] **Step 4: Implement `fzf_transform_new_journey()` — new journey wizard steps**

```rust
fn fzf_transform_new_journey(
    home: &Path,
    cwd: &Path,
    exe: &str,
    cwd_str: &str,
    reload_list: &str,
    rest: &str,
    query: &str,
) -> Result<String> {
    let parts: Vec<&str> = rest.splitn(3, ':').collect();

    match parts.first().copied() {
        // Step 1: title entered → move to description
        Some("title") => {
            if query.trim().is_empty() {
                return Ok("change-header(Title is required — type a title and press Enter)".to_string());
            }
            let title = query.trim();
            let escaped_title = title.replace('\'', "'\\''");
            let item_key = format!("new:desc:{escaped_title}");
            Ok(format!(
                "reload(echo '{item_key}\tPress Enter to skip')+change-prompt(Description> )+change-header(Optional description | type + enter, or just enter to skip | esc: back)+clear-query"
            ))
        }
        // Step 2: description entered → create journey, then offer worktree or return to list
        Some("desc") => {
            let title = parts.get(1).unwrap_or(&"");
            let desc = if query.is_empty() { "" } else { query };
            let dispatch_query = if desc.is_empty() {
                title.to_string()
            } else {
                format!("{title}\t{desc}")
            };
            let dispatch = format!(
                "{exe} __fzf-dispatch _ new-journey --query={} --cwd={cwd_str} 2>&1",
                shell_quote_value(&dispatch_query)
            );

            // Check if we're in a git repo — if so, offer worktree action
            if let Ok(repo) = git::discover_repo(cwd) {
                let slug = storage::slugify(title);
                let jid = slug.clone(); // approximate; the real ID is allocated by create_journey
                // We create the journey first, then offer worktree linking
                let repo_name = repo.root.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "repo".to_string());
                // We need to create journey first, so use execute-silent, then reload with worktree options
                let items = format!(
                    "new:wt:{{jid}}:attach\tAttach current worktree\nnew:wt:{{jid}}:create\tCreate a new worktree\nnew:wt:{{jid}}:skip\tSkip"
                );
                // Actually, we need the real journey ID from the dispatch output.
                // This is tricky — we'll use a two-phase approach: create the journey via dispatch,
                // parse the ID from output, and reload with worktree options.
                // Simpler: just create and return to list with header notification.
                let dispatch_silent = format!(
                    "{exe} __fzf-dispatch _ new-journey --query={} --cwd={cwd_str} 2>/dev/null",
                    shell_quote_value(&dispatch_query)
                );
                let preview = format!("{exe} __fzf-preview {{1}}");
                return Ok(format!(
                    "execute-silent({dispatch_silent})+reload({reload_list})+change-prompt(Journeys> )+transform-header({dispatch})+change-preview({preview})+clear-query+enable-search"
                ));
            }

            // No git repo — just create and return
            let dispatch_silent = format!(
                "{exe} __fzf-dispatch _ new-journey --query={} --cwd={cwd_str} 2>/dev/null",
                shell_quote_value(&dispatch_query)
            );
            let preview = format!("{exe} __fzf-preview {{1}}");
            Ok(format!(
                "execute-silent({dispatch_silent})+reload({reload_list})+change-prompt(Journeys> )+transform-header({dispatch})+change-preview({preview})+clear-query+enable-search"
            ))
        }
        _ => Ok(String::new()),
    }
}
```

- [ ] **Step 5: Add `shell_quote_value()` helper**

```rust
fn shell_quote_value(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
```

- [ ] **Step 6: Build and verify**

Run: `cargo build 2>&1`
Expected: successful build

- [ ] **Step 7: Commit**

```bash
git add src/app.rs
git commit -m "feat: implement transform logic for Enter across all TUI modes

Handles list→actions, action dispatch, worktree wizard steps,
unlink picker, and new journey wizard."
```

---

### Task 3: Implement transform logic for Esc (back navigation)

**Files:**
- Modify: `src/app.rs` (add `fzf_transform_esc()`)

**Interfaces:**
- Consumes: `fzf_transform()` from Task 1
- Produces: `fzf_transform_esc()` — returns fzf actions for Esc in each mode

- [ ] **Step 1: Implement `fzf_transform_esc()`**

```rust
fn fzf_transform_esc(
    exe: &str,
    reload_list: &str,
    item: &str,
) -> Result<String> {
    let list_header = "Journey list | enter: actions | ctrl-n: new journey | ctrl-r: reload";
    let preview = format!("{exe} __fzf-preview {{1}}");
    let back_to_list = format!(
        "reload({reload_list})+change-prompt(Journeys> )+change-header({list_header})+change-preview({preview})+clear-query+enable-search"
    );

    // In list mode → abort (exit fzf)
    if !item.contains(':') {
        return Ok("abort".to_string());
    }

    // In action menu → back to list
    if item.starts_with("act:") {
        return Ok(back_to_list);
    }

    // In unlink picker → back to action menu for that journey
    if let Some(rest) = item.strip_prefix("repo:") {
        let journey_id = rest.split(':').next().unwrap_or("");
        let reload_actions = format!("{exe} __fzf-action-items {journey_id}");
        let preview = format!("{exe} __fzf-preview {journey_id}");
        return Ok(format!(
            "reload({reload_actions})+change-prompt(Action> )+change-header(Actions for {journey_id} | esc: back)+change-preview({preview})+clear-query"
        ));
    }

    // In worktree wizard → back to action menu
    if let Some(rest) = item.strip_prefix("wiz:") {
        let journey_id = rest.split(':').next().unwrap_or("");
        let parts: Vec<&str> = rest.splitn(4, ':').collect();
        match parts.get(1).copied() {
            // From path step → back to branch step
            Some("path") => {
                let branch = parts.get(2).unwrap_or(&"");
                let slug = storage::slugify(journey_id);
                let item_key = format!("wiz:{journey_id}:branch:{slug}");
                return Ok(format!(
                    "reload(echo '{item_key}\tDefault: {slug}')+change-prompt(Branch> )+change-header(New branch (default: {slug}) | type name + enter | esc: back)+clear-query"
                ));
            }
            // From branch step → back to action menu
            _ => {
                let reload_actions = format!("{exe} __fzf-action-items {journey_id}");
                let preview = format!("{exe} __fzf-preview {journey_id}");
                return Ok(format!(
                    "reload({reload_actions})+change-prompt(Action> )+change-header(Actions for {journey_id} | esc: back)+change-preview({preview})+clear-query"
                ));
            }
        }
    }

    // In new journey wizard → back to list
    if item.starts_with("new:") {
        let parts: Vec<&str> = item.strip_prefix("new:").unwrap_or("").splitn(3, ':').collect();
        match parts.first().copied() {
            // From desc step → back to title step
            Some("desc") => {
                return Ok(format!(
                    "reload(echo 'new:title\tType title and press Enter')+change-prompt(Title> )+change-header(New journey | type a title + enter | esc: cancel)+clear-query"
                ));
            }
            // From title step or anything else → back to list
            _ => return Ok(back_to_list),
        }
    }

    Ok(back_to_list)
}
```

- [ ] **Step 2: Build and verify**

Run: `cargo build 2>&1`
Expected: successful build

- [ ] **Step 3: Commit**

```bash
git add src/app.rs
git commit -m "feat: implement Esc transform for back navigation in all TUI modes

Each mode navigates back one level: wizard→action menu→list→exit."
```

---

### Task 4: Rewrite `run_journey_list()` to use transform binds

**Files:**
- Modify: `src/picker.rs:39-111` (rewrite `run_journey_list()`)

**Interfaces:**
- Consumes: `__fzf-transform`, `__fzf-candidates`, `__fzf-preview`, `__fzf-action-items` subcommands
- Produces: single fzf window with transform-based mode switching

The main fzf now uses `transform` binds for Enter and Esc instead of `execute()`:

- [ ] **Step 1: Rewrite `run_journey_list()`**

Replace the function body in `src/picker.rs`:

```rust
pub fn run_journey_list(
    default_filter: JourneyStatus,
    rows: &[IndexEntry],
    cwd: &Path,
) -> Result<()> {
    ensure_fzf()?;

    let default_query = default_filter.to_string();
    let input = candidate_lines(rows, Some(&default_query));
    let exe = shell_quote(&env::current_exe().context("failed to resolve current executable")?);
    let cwd_str = shell_quote(cwd);

    let preview_command = format!("{exe} __fzf-preview {{1}}");
    let reload_command = format!("{exe} __fzf-candidates --query={{q}}");

    // Transform binds: the Rust binary decides what fzf does on Enter/Esc
    let enter_bind = format!(
        "enter:transform:{exe} __fzf-transform enter {{1}} --query={{q}} --cwd={cwd_str}"
    );
    let esc_bind = format!(
        "esc:transform:{exe} __fzf-transform esc {{1}} --query={{q}} --cwd={cwd_str}"
    );
    let refresh_bind = format!("ctrl-r:reload({reload_command})");
    let change_bind = format!("change:reload({reload_command})");

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

    // ctrl-n starts the new journey wizard inline
    let new_title_items = "new:title\tType title and press Enter";
    let new_bind = format!(
        "ctrl-n:reload(echo '{new_title_items}')+change-prompt(Title> )+change-header(New journey | type a title + enter | esc: cancel)+clear-query+disable-search"
    );

    let header = format!(
        "Journey list | enter: actions | ctrl-n: new journey{git_hint} | ctrl-r: reload"
    );

    let mut child = Command::new("fzf")
        .arg("--ansi")
        .arg("--disabled")
        .arg("--no-multi")
        .arg("--cycle")
        .arg("--border")
        .arg("--layout=reverse")
        .arg("--height=100%")
        .arg(format!("--prompt=Journeys [{default_filter}]> "))
        .arg(format!("--query={default_query}"))
        .arg(format!("--header={header}"))
        .arg(format!("--delimiter={}", "\t"))
        .arg("--with-nth=2..")
        .arg("--preview-window=right:60%:wrap")
        .arg(format!("--preview={preview_command}"))
        .arg(format!("--bind={enter_bind}"))
        .arg(format!("--bind={esc_bind}"))
        .arg(format!("--bind={refresh_bind}"))
        .arg(format!("--bind={change_bind}"))
        .arg(format!("--bind={new_bind}"))
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to start fzf")?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open fzf stdin"))?;
        stdin.write_all(input.as_bytes())?;
    }

    let status = child.wait()?;
    if status.success() || matches!(status.code(), Some(1 | 130)) {
        Ok(())
    } else {
        bail!("fzf exited with status {status}");
    }
}
```

- [ ] **Step 2: Build and verify**

Run: `cargo build 2>&1`
Expected: successful build

- [ ] **Step 3: Manual smoke test**

Run: `cargo run` in a directory with existing journeys.

Verify:
1. Journey list appears with preview on right
2. Press Enter on a journey → left side shows action menu, preview stays
3. Press Esc from action menu → back to journey list
4. Select "Resume" → header shows result, returns to list
5. Select "cd journey" → shell opens, `exit` returns to list
6. Select "New branch + worktree" → branch prompt appears, enter → path prompt → creates worktree
7. Esc from branch prompt → back to action menu
8. Select "Unlink a repo" → repo picker appears, select one → unlinks, returns to list
9. ctrl-n → title prompt → enter title → desc prompt → creates journey, returns to list

- [ ] **Step 4: Commit**

```bash
git add src/picker.rs
git commit -m "feat: rewrite run_journey_list to use transform binds

Single fzf window for all interactions. Enter/Esc behavior changes
based on item prefix. Preview pane always visible."
```

---

### Task 5: Remove old multi-window functions and update preview handler

**Files:**
- Modify: `src/picker.rs` (remove `pick_journey_action`, `fzf_notify`, `fzf_prompt_text`, `fzf_pick_worktree_action`, `run_new_journey`, `pick_repo_to_unlink`, `pick_new_worktree`, `NewJourneyInput`, `WorktreeAction`, `NewWorktreeInput`)
- Modify: `src/app.rs` (remove `fzf_action_menu`, `fzf_new_journey`, `notify_action`, `pick_and_unlink_repo`, `create_and_link_worktree`, `open_shell_in_journey`; remove old `FzfActionMenu`/`FzfNewJourney` dispatch arms)
- Modify: `src/cli.rs` (remove `FzfActionMenu` and `FzfNewJourney` variants)
- Modify: `src/picker.rs` tests (update test to reference `JOURNEY_ACTIONS` if still needed, or remove)

**Interfaces:**
- Consumes: nothing (cleanup task)
- Produces: cleaner codebase with no dead code

- [ ] **Step 1: Remove old CLI variants**

In `src/cli.rs`, remove:
```rust
    #[command(name = "__fzf-action-menu", hide = true)]
    FzfActionMenu { id: String },
    #[command(name = "__fzf-new-journey", hide = true)]
    FzfNewJourney {
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
```

- [ ] **Step 2: Remove old dispatch arms in `run()`**

In `src/app.rs`, remove:
```rust
        Some(Commands::FzfActionMenu { id }) => {
            fzf_action_menu(&home, &cwd, &id)?;
            Ok(String::new())
        }
        Some(Commands::FzfNewJourney { cwd: override_cwd }) => {
            let effective_cwd = override_cwd.unwrap_or_else(|| cwd.clone());
            fzf_new_journey(&home, &effective_cwd)
        }
```

- [ ] **Step 3: Remove old functions from app.rs**

Remove these functions entirely from `src/app.rs`:
- `fzf_action_menu()`
- `fzf_new_journey()`
- `notify_action()`
- `pick_and_unlink_repo()`
- `create_and_link_worktree()`
- `open_shell_in_journey()`

- [ ] **Step 4: Remove old functions and types from picker.rs**

Remove these from `src/picker.rs`:
- `pub enum WorktreeAction`
- `pub struct NewJourneyInput`
- `pub struct NewWorktreeInput`
- `pub fn pick_journey_action()`
- `pub fn fzf_notify()`
- `pub fn fzf_prompt_text()`
- `pub fn fzf_pick_worktree_action()`
- `pub fn run_new_journey()`
- `pub fn pick_repo_to_unlink()`
- `pub fn pick_new_worktree()`
- The `JOURNEY_ACTIONS` const (moved to `fzf_action_items()` in app.rs)

Keep:
- `run_journey_list()` (rewritten in Task 4)
- `candidate_lines()`, `matches_query()`
- `preview_for_id()`, `build_preview()`, `render_docs()`
- `ensure_fzf()`, `shell_quote()`, `sanitize_item()`
- `styled_status()`, `label()`, `color()`

- [ ] **Step 5: Update picker.rs test**

Remove or update the test `journey_action_menu_opens_shell_first` since `JOURNEY_ACTIONS` has moved. If the const is no longer in picker.rs, remove the test. Add a test in app.rs instead:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fzf_action_items_format() {
        let output = fzf_action_items("test-journey").unwrap();
        assert!(output.starts_with("act:test-journey:shell\t"));
        assert!(output.contains("act:test-journey:worktree\t"));
    }
}
```

- [ ] **Step 6: Build and verify no dead code warnings**

Run: `cargo build 2>&1`
Expected: clean build with no warnings about unused functions

Run: `cargo test 2>&1`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/cli.rs src/app.rs src/picker.rs
git commit -m "refactor: remove old multi-window fzf functions

All interactions now happen inline via transform binds.
Removed: pick_journey_action, fzf_notify, fzf_prompt_text,
fzf_pick_worktree_action, run_new_journey, pick_repo_to_unlink,
pick_new_worktree, fzf_action_menu, fzf_new_journey."
```

---

### Task 6: Enhance preview for wizard and new-journey modes

**Files:**
- Modify: `src/app.rs` (update `preview_for_id` dispatch or add `__fzf-preview` handling for prefixed items)
- Modify: `src/picker.rs` (add `preview_for_wizard_item()`)

**Interfaces:**
- Consumes: `preview_for_id()` from picker.rs
- Produces: enriched preview that shows wizard state alongside journey details

Currently `__fzf-preview` receives `{1}` which is the first tab-separated field. In action/wizard/new modes, this is the prefixed item key. The preview handler needs to extract the journey ID and optionally show wizard state.

- [ ] **Step 1: Update preview dispatch in `run()` to handle prefixed items**

In `src/app.rs`, change the `FzfPreview` handler:

```rust
        Some(Commands::FzfPreview { id }) => fzf_preview(&home, &id),
```

Add:
```rust
fn fzf_preview(home: &Path, raw_id: &str) -> Result<String> {
    // Extract journey ID from prefixed item keys
    let journey_id = if let Some(rest) = raw_id.strip_prefix("act:") {
        rest.split(':').next().unwrap_or(rest)
    } else if let Some(rest) = raw_id.strip_prefix("repo:") {
        rest.split(':').next().unwrap_or(rest)
    } else if let Some(rest) = raw_id.strip_prefix("wiz:") {
        rest.split(':').next().unwrap_or(rest)
    } else if raw_id.starts_with("new:") {
        // New journey has no ID yet — show placeholder
        return Ok(picker::build_new_journey_preview(raw_id));
    } else {
        raw_id
    };
    picker::preview_for_id(home, journey_id)
}
```

- [ ] **Step 2: Add `build_new_journey_preview()` in picker.rs**

```rust
pub fn build_new_journey_preview(item_key: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}", color(style("New Journey").cyan().bold()));
    out.push('\n');

    let rest = item_key.strip_prefix("new:").unwrap_or(item_key);
    let parts: Vec<&str> = rest.splitn(3, ':').collect();
    match parts.first().copied() {
        Some("title") => {
            let _ = writeln!(out, "{} {}", label("step:"), "Enter a title");
            let _ = writeln!(out, "\n{}", color(style("Type your journey title in the search field and press Enter.").dim()));
        }
        Some("desc") => {
            let title = parts.get(1).unwrap_or(&"");
            let _ = writeln!(out, "{} {}", label("title:"), title);
            let _ = writeln!(out, "{} {}", label("step:"), "Enter a description (optional)");
            let _ = writeln!(out, "\n{}", color(style("Type a description or press Enter to skip.").dim()));
        }
        _ => {
            let _ = writeln!(out, "{}", color(style("Setting up new journey...").dim()));
        }
    }
    out
}
```

- [ ] **Step 3: Build and verify**

Run: `cargo build 2>&1`
Expected: successful build

- [ ] **Step 4: Commit**

```bash
git add src/app.rs src/picker.rs
git commit -m "feat: preview handler supports prefixed items and new-journey wizard

Preview extracts journey ID from act:/repo:/wiz: prefixes.
New journey wizard shows progressive preview as user fills in fields."
```

---

### Task 7: Handle `disable-search` / `enable-search` for mode transitions

**Files:**
- Modify: `src/app.rs` (ensure all transform outputs include search toggling)

**Interfaces:**
- Consumes: all transform functions from Tasks 2-3
- Produces: consistent search behavior — enabled in list mode, disabled in action/wizard/picker modes

The main fzf starts with `--disabled` (search disabled). In the current code, `change:reload(...)` fires on query changes to do server-side filtering. When we switch to action/wizard mode, we need to disable this to prevent the query from filtering action items. When returning to list, re-enable it.

- [ ] **Step 1: Audit all transform outputs for search toggling**

Review every `fzf_transform_enter()` and `fzf_transform_esc()` return value:

For transitions TO action/wizard/picker mode: ensure `disable-search` is present (the `change` bind won't fire reloads while in these modes).

For transitions BACK to list mode: ensure `enable-search` is present.

Note: fzf's `--disabled` starts search disabled. Our `change:reload(...)` bind uses the query for server-side filtering. When we `enable-search`, the `change` bind starts firing. When we `disable-search`, it stops. This is what we want: live filtering in list mode, static items in other modes.

- [ ] **Step 2: Verify the `change` bind only applies in list mode**

The `change:reload(...)` bind fires whenever the query changes. In action/wizard modes with `disable-search`, the query doesn't trigger filtering. But we use the query field for text input in wizard modes. We need the query to NOT trigger reload in those modes.

Since `disable-search` prevents the search from being active, the `change` event should not fire. Verify this works correctly during manual testing.

- [ ] **Step 3: Build and test**

Run: `cargo build 2>&1`
Expected: clean build

Manual test: navigate between modes, verify no spurious reloads in wizard mode.

- [ ] **Step 4: Commit (if changes were needed)**

```bash
git add src/app.rs
git commit -m "fix: ensure search toggling is consistent across mode transitions"
```

---

### Task 8: Final integration test and edge case fixes

**Files:**
- Modify: `src/app.rs`, `src/picker.rs` as needed for bug fixes

**Interfaces:**
- Consumes: entire single-window TUI
- Produces: polished, working TUI

- [ ] **Step 1: Build and run all tests**

Run: `cargo build 2>&1 && cargo test 2>&1`
Expected: all pass

- [ ] **Step 2: Manual end-to-end test**

Test each flow:
1. Launch `journey list` → journey list with preview ✓
2. Enter on journey → action menu, preview stays ✓
3. Esc → back to list ✓
4. Enter → Resume → header shows result, back to list ✓
5. Enter → cd journey → shell opens, exit → back to list ✓
6. Enter → New branch + worktree → branch prompt → path prompt → creates worktree ✓
7. Esc from path step → branch step ✓
8. Esc from branch step → action menu ✓
9. Enter → Unlink a repo → repo picker → select → unlinks, back to list ✓
10. Esc from unlink picker → action menu ✓
11. ctrl-n → title prompt → enter → desc prompt → enter → journey created ✓
12. Esc from desc prompt → title prompt ✓
13. Esc from title prompt → list ✓
14. Enter → Link current worktree → header shows result ✓
15. Enter → Status → header shows status ✓
16. Enter → Pause → header shows result ✓

- [ ] **Step 3: Fix any issues found**

Address each failing case individually.

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat: single-window TUI — all interactions inline in one fzf instance

Journey list, action menu, worktree wizard, unlink picker, and
new journey flow all happen in the same fzf window. Preview pane
stays visible throughout. Esc navigates back, Enter advances."
```
