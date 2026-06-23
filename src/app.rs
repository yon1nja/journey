use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use console::style;

use crate::cli::{join_words, Cli, Commands, DocCommands};
use crate::events;
use crate::git;
use crate::models::{EventKind, IndexEntry, JourneyStatus, RepoRef};
use crate::picker;
use crate::storage::{self, JourneyContext};

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
        Some(Commands::FzfCandidates { query }) => fzf_candidates(&home, query.as_deref()),
        Some(Commands::FzfPreview { id }) => picker::preview_for_id(&home, &id),
        Some(Commands::FzfActionMenu { id }) => {
            fzf_action_menu(&home, &cwd, &id)?;
            Ok(String::new())
        }
        Some(Commands::FzfNewJourney { cwd: override_cwd }) => {
            let effective_cwd = override_cwd.unwrap_or_else(|| cwd.clone());
            fzf_new_journey(&home, &effective_cwd)
        }
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

fn ui_active(value: &str) -> String {
    style(value).cyan().bold().force_styling(true).to_string()
}

fn ui_success(value: &str) -> String {
    style(value).green().bold().force_styling(true).to_string()
}

fn ui_dim(value: &str) -> String {
    style(value).dim().force_styling(true).to_string()
}

fn ui_label(value: &str) -> String {
    ui_dim(value)
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

    if !non_interactive && io::stdout().is_terminal() {
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

fn fzf_candidates(home: &Path, query: Option<&str>) -> Result<String> {
    storage::ensure_home(home)?;
    let index = storage::load_index(home)?;
    let mut rows = index.journeys.into_iter().collect::<Vec<_>>();
    rows.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
    Ok(picker::candidate_lines(&rows, query))
}

fn fzf_action_menu(home: &Path, cwd: &Path, journey_id: &str) -> Result<()> {
    let Some(action) = picker::pick_journey_action(journey_id)? else {
        return Ok(());
    };

    match action.as_str() {
        "resume" => print_action_output(resume(home, cwd, Some(journey_id))?),
        "status" => print_action_output(status(home, cwd, Some(journey_id))?),
        "shell" => open_shell_in_journey(home, journey_id),
        "path" => print_action_output(storage::journey_dir(home, journey_id).display().to_string()),
        "pause" => print_action_output(set_status(
            home,
            cwd,
            Some(journey_id),
            JourneyStatus::Paused,
        )?),
        "archive" => print_action_output(set_status(
            home,
            cwd,
            Some(journey_id),
            JourneyStatus::Archived,
        )?),
        "abandon" => print_action_output(set_status(
            home,
            cwd,
            Some(journey_id),
            JourneyStatus::Abandoned,
        )?),
        _ => bail!("unknown fzf action: {action}"),
    }
}

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

fn print_action_output(output: String) -> Result<()> {
    if !output.is_empty() {
        println!("{output}");
    }
    wait_for_enter()
}

fn wait_for_enter() -> Result<()> {
    print!("\nPress Enter to return to Journey list...");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(())
}

fn open_shell_in_journey(home: &Path, journey_id: &str) -> Result<()> {
    let dir = storage::journey_dir(home, journey_id);
    if !dir.exists() {
        bail!("Journey folder does not exist: {}", dir.display());
    }

    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let status = Command::new(&shell)
        .current_dir(&dir)
        .status()
        .with_context(|| format!("failed to open shell `{shell}` in {}", dir.display()))?;

    if !status.success() {
        bail!("shell exited with status {status}");
    }
    Ok(())
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
