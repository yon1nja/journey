use std::env;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::cli::{join_words, Cli, Commands, DocCommands};
use crate::events;
use crate::git;
use crate::models::{EventKind, IndexEntry, JourneyStatus, RepoRef};
use crate::picker;
use crate::storage::{self, JourneyContext};

const SHELL_INTEGRATION_ENV: &str = "JOURNEY_SHELL_INTEGRATION";
const SHELL_CD_PREFIX: &str = "__journey_cd__\t";

pub fn run(cli: Cli) -> Result<String> {
    let home = storage::journey_home()?;
    let cwd = env::current_dir().context("failed to read current directory")?;

    match cli.command {
        None => list_journeys(&home, &cwd, JourneyStatus::Active, false),
        Some(Commands::New(args)) => new_journey(&home, &join_words(&args.text), args.description),
        Some(Commands::Link {
            repo_path,
            name,
            journey,
        }) => link_repo(&home, &cwd, journey.as_deref(), &repo_path, name),
        Some(Commands::Unlink { repo_name, journey }) => {
            unlink_repo(&home, &cwd, journey.as_deref(), &repo_name)
        }
        Some(Commands::Resume(args)) => resume(&home, &cwd, args.id.as_deref()),
        Some(Commands::List {
            status,
            non_interactive,
        }) => list_journeys(
            &home,
            &cwd,
            status.unwrap_or(JourneyStatus::Active),
            non_interactive,
        ),
        Some(Commands::ShellInit) => shell_init(),
        Some(Commands::FzfCandidates { query }) => fzf_candidates(&home, query.as_deref()),
        Some(Commands::FzfPreview { id }) => fzf_preview(&home, &id),
        Some(Commands::Status { id }) => status(&home, &cwd, id.as_deref()),
        Some(Commands::Doc { command }) => doc_command(&home, &cwd, command),
        Some(Commands::Doctor { repair }) => doctor(&home, repair),
        Some(Commands::Pause(args)) => {
            set_status(&home, &cwd, args.id.as_deref(), JourneyStatus::Paused)
        }
        Some(Commands::Archive(args)) => {
            set_status(&home, &cwd, args.id.as_deref(), JourneyStatus::Archived)
        }
        Some(Commands::Abandon(args)) => {
            set_status(&home, &cwd, args.id.as_deref(), JourneyStatus::Abandoned)
        }
        Some(Commands::FzfActionItems { id }) => fzf_action_items(&id),
        Some(Commands::FzfDispatch {
            id,
            action,
            query,
            cwd: override_cwd,
        }) => {
            let effective_cwd = override_cwd.unwrap_or_else(|| cwd.clone());
            fzf_dispatch(&home, &effective_cwd, &id, &action, query.as_deref())
        }
        Some(Commands::FzfTransform {
            event,
            item,
            query,
            cwd: override_cwd,
        }) => {
            let effective_cwd = override_cwd.unwrap_or_else(|| cwd.clone());
            fzf_transform(&home, &effective_cwd, &event, &item, query.as_deref())
        }
    }
}

fn new_journey(home: &Path, title: &str, description: Option<String>) -> Result<String> {
    let now = events::now_rfc3339()?;
    let ctx = storage::create_journey(home, title, clean_optional(description), &now)?;
    Ok(format!(
        "created Journey `{}` at {}",
        ctx.journey.id,
        ctx.path.display()
    ))
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn list_journeys(
    home: &Path,
    cwd: &Path,
    default_filter: JourneyStatus,
    non_interactive: bool,
) -> Result<String> {
    storage::ensure_home(home)?;
    let index = storage::load_index(home)?;
    let mut rows = index.journeys.into_iter().collect::<Vec<_>>();
    rows.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));

    if rows.is_empty() {
        return Ok("no Journeys".to_string());
    }

    if !non_interactive
        && (io::stdout().is_terminal()
            || (shell_integration_active() && io::stderr().is_terminal()))
    {
        picker::run_journey_list(default_filter, &rows, cwd)?;
        return Ok(String::new());
    }

    let rows = rows
        .into_iter()
        .filter(|entry| entry.status == default_filter)
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Ok(format!("no {default_filter} Journeys"));
    }

    render_journey_table(&rows)
}

fn render_journey_table(rows: &[IndexEntry]) -> Result<String> {
    let mut out = String::new();
    for entry in rows {
        let repos = if entry.repos.is_empty() {
            "no repos".to_string()
        } else {
            entry.repos.join(", ")
        };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            entry.id, entry.status, entry.updated, repos
        ));
    }
    Ok(out.trim_end().to_string())
}

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

fn fzf_candidates(home: &Path, query: Option<&str>) -> Result<String> {
    storage::ensure_home(home)?;
    let index = storage::load_index(home)?;
    let mut rows = index.journeys.into_iter().collect::<Vec<_>>();
    rows.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
    Ok(picker::candidate_lines(&rows, query))
}

fn status(home: &Path, cwd: &Path, id: Option<&str>) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let events = events::read_events(&ctx.path)?;

    let mut out = String::new();
    out.push_str(&format!("{}\n", ctx.journey.title));
    if let Some(description) = &ctx.journey.description {
        out.push_str(&format!("description: {description}\n"));
    }
    out.push_str(&format!("id: {}\n", ctx.journey.id));
    out.push_str(&format!("status: {}\n", ctx.journey.status));
    out.push_str(&format!("path: {}\n", ctx.path.display()));
    out.push_str(&format!("repos: {}\n", ctx.journey.repos.len()));
    out.push_str(&format!("events: {}\n", events.len()));
    Ok(out.trim_end().to_string())
}

fn link_repo(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    repo_path: &Path,
    name: Option<String>,
) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    let discovered = git::discover_repo(repo_path)?;
    let repo_name = match name {
        Some(name) => validate_name(&name)?,
        None => discovered
            .root
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "could not derive repo name from {}",
                    discovered.root.display()
                )
            })?,
    };

    if ctx.journey.repos.iter().any(|repo| repo.name == repo_name) {
        bail!("repo name `{repo_name}` is already linked; pass --name to choose another name");
    }

    let now = events::now_rfc3339()?;
    let canonical_root = storage::canonicalize_existing(&discovered.root)?;
    let canonical_worktree =
        storage::attach_worktree(home, &ctx.journey.id, &repo_name, &canonical_root, &now)?;
    let repo = RepoRef {
        name: repo_name.clone(),
        root: canonical_root,
        worktree: canonical_worktree,
        branch: discovered.branch.clone(),
    };
    ctx.journey.repos.push(repo.clone());
    if let Err(err) = storage::save_journey(&ctx.path, &ctx.journey) {
        let _ = storage::detach_worktree(home, &ctx.journey.id, &repo.name);
        return Err(err);
    }
    if let Err(err) = storage::sync_worktree_link(&ctx.path, &repo) {
        let _ = storage::detach_worktree(home, &ctx.journey.id, &repo.name);
        return Err(err);
    }

    let record = events::append_event(
        &ctx.path,
        EventKind::LinkRepo {
            name: repo.name.clone(),
            root: repo.root.clone(),
            worktree: repo.worktree.clone(),
            branch: repo.branch.clone(),
        },
    )?;
    finish_mutation(&ctx, Some(record.ts.clone()))?;

    Ok(format!(
        "linked `{}` ({}) to Journey `{}`",
        repo.name,
        repo.worktree.display(),
        ctx.journey.id
    ))
}

fn unlink_repo(home: &Path, cwd: &Path, id: Option<&str>, repo_name: &str) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    let Some(position) = ctx
        .journey
        .repos
        .iter()
        .position(|repo| repo.name == repo_name)
    else {
        bail!(
            "repo `{repo_name}` is not linked to Journey `{}`",
            ctx.journey.id
        );
    };

    let repo = ctx.journey.repos.remove(position);
    storage::save_journey(&ctx.path, &ctx.journey)?;
    storage::detach_worktree(home, &ctx.journey.id, &repo.name)?;
    storage::remove_worktree_link(&ctx.path, &repo.name)?;

    let record = events::append_event(
        &ctx.path,
        EventKind::UnlinkRepo {
            name: repo.name.clone(),
            root: repo.root.clone(),
            worktree: repo.worktree.clone(),
            branch: repo.branch.clone(),
        },
    )?;
    finish_mutation(&ctx, Some(record.ts))?;

    Ok(format!(
        "unlinked `{}` from Journey `{}`",
        repo.name, ctx.journey.id
    ))
}

fn resume(home: &Path, cwd: &Path, id: Option<&str>) -> Result<String> {
    set_status(home, cwd, id, JourneyStatus::Active)
}

fn doc_command(home: &Path, cwd: &Path, command: DocCommands) -> Result<String> {
    match command {
        DocCommands::New { name, journey } => {
            let ctx = storage::resolve_context(home, journey.as_deref(), cwd)?;
            fs::create_dir_all(ctx.path.join(storage::DOCS_DIR))?;
            let path = storage::doc_path(&ctx.path, &name)?;
            if path.exists() {
                bail!("doc already exists: {}", path.display());
            }
            let title = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().replace('-', " "))
                .unwrap_or_else(|| "doc".to_string());
            storage::write_string_atomic(&path, &format!("# {title}\n\n"))?;
            let now = events::now_rfc3339()?;
            storage::update_index_entry(home, &ctx.journey, &now)?;
            Ok(path.display().to_string())
        }
        DocCommands::List { journey } => {
            let ctx = storage::resolve_context(home, journey.as_deref(), cwd)?;
            let docs_dir = ctx.path.join(storage::DOCS_DIR);
            if !docs_dir.exists() {
                return Ok("no docs".to_string());
            }
            let mut docs = fs::read_dir(&docs_dir)?
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let path = entry.path();
                    if path.is_file() {
                        path.file_name()
                            .map(|name| name.to_string_lossy().to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            docs.sort();
            if docs.is_empty() {
                Ok("no docs".to_string())
            } else {
                Ok(docs.join("\n"))
            }
        }
        DocCommands::Path { name, journey } => {
            let ctx = storage::resolve_context(home, journey.as_deref(), cwd)?;
            Ok(storage::doc_path(&ctx.path, &name)?.display().to_string())
        }
    }
}

fn doctor(home: &Path, repair: bool) -> Result<String> {
    storage::ensure_home(home)?;
    if repair {
        let now = events::now_rfc3339()?;
        let index = storage::rebuild_worktree_index(home, &now)?;
        return Ok(format!(
            "rebuilt worktree index with {} attachments",
            index.attachments.len()
        ));
    }

    let index = storage::load_worktree_index(home)?;
    let mut issues = Vec::new();
    for attachment in &index.attachments {
        if !attachment.worktree.exists() {
            issues.push(format!(
                "missing worktree: {} attached to Journey `{}` as `{}`",
                attachment.worktree.display(),
                attachment.journey_id,
                attachment.repo_name
            ));
        }

        let journey_path = storage::journey_dir(home, &attachment.journey_id);
        if !journey_path.join(storage::JOURNEY_FILE).exists() {
            issues.push(format!(
                "missing Journey `{}` for worktree {}",
                attachment.journey_id,
                attachment.worktree.display()
            ));
            continue;
        }

        let journey = storage::load_journey(&journey_path)?;
        let Some(repo) = journey
            .repos
            .iter()
            .find(|repo| repo.name == attachment.repo_name)
        else {
            issues.push(format!(
                "Journey `{}` does not link repo `{}` from worktree index",
                attachment.journey_id, attachment.repo_name
            ));
            continue;
        };

        match storage::canonicalize_existing(&repo.worktree) {
            Ok(path) if path == attachment.worktree => {}
            Ok(path) => issues.push(format!(
                "worktree mismatch for Journey `{}` repo `{}`: index has {}, journey.yaml has {}",
                attachment.journey_id,
                attachment.repo_name,
                attachment.worktree.display(),
                path.display()
            )),
            Err(err) => issues.push(format!(
                "cannot canonicalize Journey `{}` repo `{}` worktree {}: {err}",
                attachment.journey_id,
                attachment.repo_name,
                repo.worktree.display()
            )),
        }
    }

    if issues.is_empty() {
        Ok(format!(
            "doctor: ok ({} worktree attachments)",
            index.attachments.len()
        ))
    } else {
        let mut out = format!("doctor found {} issues:\n", issues.len());
        for issue in issues {
            out.push_str(&format!("- {issue}\n"));
        }
        out.push_str("run `journey doctor --repair` to rebuild the worktree index\n");
        Ok(out.trim_end().to_string())
    }
}

fn set_status(home: &Path, cwd: &Path, id: Option<&str>, status: JourneyStatus) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    let previous_status = ctx.journey.status;
    let now = events::now_rfc3339()?;
    let attached = if matches!(status, JourneyStatus::Active | JourneyStatus::Paused)
        && matches!(
            previous_status,
            JourneyStatus::Archived | JourneyStatus::Abandoned
        ) {
        storage::attach_journey_worktrees(home, &ctx.journey, &now)?
    } else {
        0
    };
    ctx.journey.status = status;
    storage::save_journey(&ctx.path, &ctx.journey)?;
    let detached = if matches!(status, JourneyStatus::Archived | JourneyStatus::Abandoned) {
        storage::detach_journey_worktrees(home, &ctx.journey.id)?
    } else {
        0
    };
    let record = events::append_event(&ctx.path, EventKind::StatusChange { status })?;
    finish_mutation(&ctx, Some(record.ts))?;
    if detached > 0 {
        Ok(format!(
            "Journey `{}` is now {} (detached {} worktrees)",
            ctx.journey.id, status, detached
        ))
    } else if attached > 0 {
        Ok(format!(
            "Journey `{}` is now {} (attached {} worktrees)",
            ctx.journey.id, status, attached
        ))
    } else {
        Ok(format!("Journey `{}` is now {}", ctx.journey.id, status))
    }
}

fn finish_mutation(ctx: &JourneyContext, updated: Option<String>) -> Result<()> {
    let updated = match updated {
        Some(updated) => updated,
        None => events::now_rfc3339()?,
    };
    storage::update_index_entry(&ctx.home, &ctx.journey, &updated)
}

fn shell_quote(path: &Path) -> String {
    let value = path.display().to_string();
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shell_quote_value(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shell_integration_active() -> bool {
    env::var_os(SHELL_INTEGRATION_ENV).is_some()
}

fn shell_init() -> Result<String> {
    let exe = env::current_exe().context("failed to resolve current executable")?;
    let exe_q = shell_quote(&exe);
    let cd_prefix_q = shell_quote_value(SHELL_CD_PREFIX);
    Ok(format!(
        r#"journey() {{
    local __journey_output __journey_status __journey_prefix
    __journey_prefix={cd_prefix_q}
    __journey_output="$(JOURNEY_SHELL_INTEGRATION=1 {exe_q} "$@")"
    __journey_status=$?
    if [ $__journey_status -eq 0 ] && [ "${{__journey_output#"$__journey_prefix"}}" != "$__journey_output" ]; then
        builtin cd -- "${{__journey_output#"$__journey_prefix"}}"
        return
    fi
    if [ -n "$__journey_output" ]; then
        printf '%s\n' "$__journey_output"
    fi
    return $__journey_status
}}"#
    ))
}

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

fn fzf_dispatch(
    home: &Path,
    cwd: &Path,
    journey_id: &str,
    action: &str,
    query: Option<&str>,
) -> Result<String> {
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
            let wt_path = std::path::PathBuf::from(path);
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
            let desc = if desc.is_empty() {
                None
            } else {
                Some(desc.to_string())
            };
            new_journey(home, title, desc)
        }
        _ => bail!("unknown dispatch action: {action}"),
    }
}

fn fzf_transform(
    home: &Path,
    cwd: &Path,
    event: &str,
    item: &str,
    query: Option<&str>,
) -> Result<String> {
    let exe = shell_quote(&env::current_exe().context("failed to resolve current executable")?);
    let cwd_str = shell_quote(cwd);
    let reload_list = format!("{exe} __fzf-candidates --query={{q}}");

    match event {
        "enter" => fzf_transform_enter(home, cwd, &exe, &cwd_str, &reload_list, item, query),
        "esc" => fzf_transform_esc(&exe, &reload_list, item),
        _ => Ok(String::new()),
    }
}

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
        let journey_id_q = shell_quote_value(journey_id);
        let reload_actions = format!("{exe} __fzf-action-items {journey_id_q}");
        let preview = format!("{exe} __fzf-preview {journey_id_q}");
        return Ok(format!(
            "reload({reload_actions})+change-prompt(Action> )+change-header(Actions for {journey_id} | esc: back)+change-preview({preview})+clear-query+disable-search"
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
        let journey_id_q = shell_quote_value(journey_id);
        let dispatch = format!(
            "{exe} __fzf-dispatch {journey_id_q} unlink --query={} --cwd={cwd_str}",
            shell_quote_value(repo_name)
        );
        return Ok(format!(
            "execute-silent({dispatch})+reload({reload_list})+change-prompt(Journeys> )+transform-header({exe} __fzf-dispatch {journey_id_q} unlink --query={} --cwd={cwd_str} 2>&1 || true)+clear-query+enable-search",
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
        // A real cd must be completed by the caller's shell integration.
        "shell" => {
            let dir = storage::journey_dir(home, journey_id);
            if shell_integration_active() {
                let cd_request = format!("{SHELL_CD_PREFIX}{}", dir.display());
                return Ok(format!(
                    "become(printf '%s\\n' {})",
                    shell_quote_value(&cd_request)
                ));
            }

            let message = format!(
                "selected Journey path: {} (enable parent-shell cd with: eval \"$(journey shell-init)\")",
                dir.display()
            );
            Ok(format!(
                "become(printf '%s\\n' {})",
                shell_quote_value(&message)
            ))
        }

        // Worktree wizard: switch to branch input step
        "worktree" => {
            let slug = storage::slugify(journey_id);
            let item_key = format!("wiz:{journey_id}:branch:{slug}");
            Ok(format!(
                "reload(echo '{item_key}\tDefault: {slug}')+change-prompt(Branch> )+change-header(New branch (default: {slug}) | type name + enter | esc: back)+clear-query+disable-search"
            ))
        }

        // Unlink: load repo list
        "unlink" => {
            let ctx = storage::resolve_context(home, Some(journey_id), cwd)?;
            if ctx.journey.repos.is_empty() {
                return Ok(
                    "change-header(No repos linked to this journey | press any key)+change-prompt(> )".to_string()
                );
            }
            let items: Vec<String> = ctx
                .journey
                .repos
                .iter()
                .map(|r| {
                    format!(
                        "repo:{journey_id}:{}\t{}  ({})",
                        r.name,
                        r.name,
                        r.worktree.display()
                    )
                })
                .collect();
            let items_str = items.join("\n");
            let journey_id_q = shell_quote_value(journey_id);
            let preview = format!("{exe} __fzf-preview {journey_id_q}");
            Ok(format!(
                "reload(echo {})+change-prompt(Unlink> )+change-header(Select repo to unlink from {journey_id} | esc: back)+change-preview({preview})+clear-query+disable-search",
                shell_quote_value(&items_str)
            ))
        }

        // All other simple actions: dispatch silently, show result in header
        _ => {
            let journey_id_q = shell_quote_value(journey_id);
            let action_q = shell_quote_value(action);
            let dispatch = format!(
                "{exe} __fzf-dispatch {journey_id_q} {action_q} --cwd={cwd_str} 2>&1 || true"
            );
            let preview = format!("{exe} __fzf-preview {{1}}");
            Ok(format!(
                "execute-silent({exe} __fzf-dispatch {journey_id_q} {action_q} --cwd={cwd_str} 2>/dev/null)+reload({reload_list})+change-prompt(Journeys> )+transform-header({dispatch})+change-preview({preview})+clear-query+enable-search"
            ))
        }
    }
}

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
            let branch = if query.is_empty() {
                default_branch
            } else {
                query
            };
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
            let path = if query.is_empty() {
                default_path
            } else {
                query
            };
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

fn fzf_transform_new_journey(
    _home: &Path,
    _cwd: &Path,
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
                return Ok(
                    "change-header(Title is required — type a title and press Enter)".to_string(),
                );
            }
            let title = query.trim();
            let escaped_title = title.replace('\'', "'\\''");
            let item_key = format!("new:desc:{escaped_title}");
            Ok(format!(
                "reload(echo '{item_key}\tPress Enter to skip')+change-prompt(Description> )+change-header(Optional description | type + enter, or just enter to skip | esc: back)+clear-query"
            ))
        }
        // Step 2: description entered → create journey, then return to list
        Some("desc") => {
            let title = parts.get(1).unwrap_or(&"");
            let desc = if query.is_empty() { "" } else { query };
            let dispatch_query = if desc.is_empty() {
                title.to_string()
            } else {
                format!("{title}\t{desc}")
            };
            let dispatch_silent = format!(
                "{exe} __fzf-dispatch _ new-journey --query={} --cwd={cwd_str} 2>/dev/null",
                shell_quote_value(&dispatch_query)
            );
            let dispatch = format!(
                "{exe} __fzf-dispatch _ new-journey --query={} --cwd={cwd_str} 2>&1",
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

fn fzf_transform_esc(exe: &str, reload_list: &str, item: &str) -> Result<String> {
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
        let journey_id_q = shell_quote_value(journey_id);
        let reload_actions = format!("{exe} __fzf-action-items {journey_id_q}");
        let preview = format!("{exe} __fzf-preview {journey_id_q}");
        return Ok(format!(
            "reload({reload_actions})+change-prompt(Action> )+change-header(Actions for {journey_id} | esc: back)+change-preview({preview})+clear-query+disable-search"
        ));
    }

    // In worktree wizard → back to action menu
    if let Some(rest) = item.strip_prefix("wiz:") {
        let journey_id = rest.split(':').next().unwrap_or("");
        let journey_id_q = shell_quote_value(journey_id);
        let parts: Vec<&str> = rest.splitn(4, ':').collect();
        match parts.get(1).copied() {
            // From path step → back to branch step
            Some("path") => {
                let slug = storage::slugify(journey_id);
                let branch = parts.get(2).copied().unwrap_or(slug.as_str());
                let item_key = format!("wiz:{journey_id}:branch:{branch}");
                return Ok(format!(
                    "reload(echo '{item_key}\tDefault: {branch}')+change-prompt(Branch> )+change-header(New branch (default: {branch}) | type name + enter | esc: back)+clear-query"
                ));
            }
            // From branch step → back to action menu
            _ => {
                let reload_actions = format!("{exe} __fzf-action-items {journey_id_q}");
                let preview = format!("{exe} __fzf-preview {journey_id_q}");
                return Ok(format!(
                    "reload({reload_actions})+change-prompt(Action> )+change-header(Actions for {journey_id} | esc: back)+change-preview({preview})+clear-query+disable-search"
                ));
            }
        }
    }

    // In new journey wizard → back to list
    if item.starts_with("new:") {
        let parts: Vec<&str> = item
            .strip_prefix("new:")
            .unwrap_or("")
            .splitn(3, ':')
            .collect();
        match parts.first().copied() {
            // From desc step → back to title step
            Some("desc") => {
                return Ok("reload(echo 'new:title\tType title and press Enter')+change-prompt(Title> )+change-header(New journey | type a title + enter | esc: cancel)+clear-query+disable-search".to_string());
            }
            // From title step or anything else → back to list
            _ => return Ok(back_to_list),
        }
    }

    Ok(back_to_list)
}

fn validate_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("name cannot be empty");
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        bail!("name must not contain path separators");
    }
    Ok(trimmed.to_string())
}

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
