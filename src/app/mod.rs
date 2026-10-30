mod report;
mod worktree;

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::cli::{join_words, Cli, Commands, DocCommands, ReadmeCommands, WorktreeCommands};
use crate::events;
use crate::git;
use crate::models::{EventKind, IndexEntry, JourneyStatus, RepoRef};

use crate::storage::{self, JourneyContext};
use report::ListFilter;

pub fn run(cli: Cli) -> Result<String> {
    let home = storage::journey_home()?;
    let cwd = env::current_dir().context("failed to read current directory")?;

    match cli.command {
        None => overview(&home),
        Some(Commands::Guide) => Ok(storage::GUIDE.trim_end().to_string()),
        Some(Commands::New(args)) => {
            report::new_journey(&home, &join_words(&args.text), args.description, args.json)
        }
        Some(Commands::Capture(args)) => {
            let text = read_capture_input(&args.text)?;
            capture(&home, &cwd, args.journey.as_deref(), &args.doc, &text)
        }
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
            all,
            search,
            json,
            non_interactive: _,
        }) => report::list_journeys(
            &home,
            &ListFilter {
                status,
                all,
                search: search.as_deref(),
            },
            json,
        ),
        Some(Commands::Status { id, json }) => report::status(&home, &cwd, id.as_deref(), json),
        Some(Commands::Doc { command }) => doc_command(&home, &cwd, command),
        Some(Commands::Readme { command }) => readme_command(&home, &cwd, command),
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
        Some(Commands::Close(args)) => close(
            &home,
            &cwd,
            args.id.as_deref(),
            &args.outcome,
            args.remove_worktrees,
        ),
        Some(Commands::Worktree { command }) => worktree_command(&home, &cwd, command),
    }
}

pub(crate) fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn read_capture_input(words: &[String]) -> Result<String> {
    let raw = if words.is_empty() {
        if io::stdin().is_terminal() {
            bail!("capture text is required; pass text or pipe stdin");
        }
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .context("failed to read capture text from stdin")?;
        input
    } else {
        join_words(words)
    };

    clean_capture_text(&raw)
}

fn clean_capture_text(value: &str) -> Result<String> {
    let text = value.trim();
    if text.is_empty() {
        bail!("capture text cannot be empty");
    }
    Ok(text.to_string())
}

pub(crate) fn capture(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    doc: &str,
    text: &str,
) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let text = clean_capture_text(text)?;
    let path = storage::doc_path(&ctx.path, doc)?;
    let existed = path.exists();
    let now = events::now_rfc3339()?;

    let mut content = String::new();
    if existed {
        content.push_str("\n\n");
    } else {
        content.push_str(&format!("# {}\n\n", doc_title(&path)));
    }
    content.push_str(&format!("## {now}\n\n{text}\n"));
    storage::append_string(&path, &content)?;
    storage::update_index_entry(home, &ctx.journey, &now)?;

    Ok(format!("captured to {}", path.display()))
}

fn doc_title(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().replace('-', " "))
        .unwrap_or_else(|| "doc".to_string())
}

const OVERVIEW: &str = "Journey: durable memory for engineering work, operated by agents.\nAgents: run `journey guide` to learn how to use it. Run `journey --help` for all commands.";

fn overview(home: &Path) -> Result<String> {
    let filter = ListFilter {
        status: None,
        all: false,
        search: None,
    };
    let list = report::list_journeys(home, &filter, false)?;
    Ok(format!("{OVERVIEW}\n\nActive Journeys:\n{list}"))
}

pub(crate) fn close(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    outcome: &str,
    remove_worktrees: bool,
) -> Result<String> {
    let outcome = outcome.trim();
    if outcome.is_empty() {
        bail!("outcome cannot be empty");
    }
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    if ctx.journey.status == JourneyStatus::Done {
        bail!(
            "Journey `{}` is already done; resume it first to close it again",
            ctx.journey.id
        );
    }

    let mut removed = 0;
    if remove_worktrees {
        for repo in &ctx.journey.repos {
            git::ensure_worktree_removable(&repo.root, &repo.worktree)?;
        }
        for repo in &ctx.journey.repos {
            git::remove_worktree(&repo.root, &repo.worktree)?;
            storage::remove_worktree_link(&ctx.path, &repo.name)?;
            removed += 1;
        }
    }

    ctx.journey.status = JourneyStatus::Done;
    ctx.journey.outcome = Some(outcome.to_string());
    storage::save_journey(&ctx.path, &ctx.journey)?;
    let detached = storage::detach_journey_worktrees(home, &ctx.journey.id)?;
    let record = events::append_event(
        &ctx.path,
        EventKind::Close {
            outcome: outcome.to_string(),
        },
    )?;
    finish_mutation(&ctx, Some(record.ts))?;

    if remove_worktrees {
        Ok(format!(
            "Journey `{}` is done (removed {removed} worktrees)",
            ctx.journey.id
        ))
    } else {
        Ok(format!(
            "Journey `{}` is done (detached {detached} worktrees)",
            ctx.journey.id
        ))
    }
}

fn worktree_command(home: &Path, cwd: &Path, command: WorktreeCommands) -> Result<String> {
    match command {
        WorktreeCommands::New {
            branch,
            from,
            repo,
            path,
            name,
            journey,
        } => create_worktree(
            home,
            cwd,
            journey.as_deref(),
            &WorktreeRequest {
                repo: repo.as_deref(),
                branch: &branch,
                from: from.as_deref(),
                path: path.as_deref(),
                name,
            },
        ),
        WorktreeCommands::Remove { name, journey } => {
            delete_worktree(home, cwd, journey.as_deref(), &name)
        }
    }
}

pub(crate) fn resume(home: &Path, cwd: &Path, id: Option<&str>) -> Result<String> {
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
        DocCommands::List { journey, json } => {
            report::list_docs(home, cwd, journey.as_deref(), json)
        }
        DocCommands::Path { name, journey } => {
            let ctx = storage::resolve_context(home, journey.as_deref(), cwd)?;
            Ok(storage::doc_path(&ctx.path, &name)?.display().to_string())
        }
    }
}

fn readme_command(home: &Path, cwd: &Path, command: ReadmeCommands) -> Result<String> {
    match command {
        ReadmeCommands::New { journey } => {
            let ctx = storage::resolve_context(home, journey.as_deref(), cwd)?;
            let path = ctx.path.join(storage::README_FILE);
            if path.exists() {
                bail!("README already exists: {}", path.display());
            }
            storage::write_string_atomic(&path, &format!("# {}\n\n", ctx.journey.title))?;
            let now = events::now_rfc3339()?;
            storage::update_index_entry(home, &ctx.journey, &now)?;
            Ok(path.display().to_string())
        }
        ReadmeCommands::Path { journey } => {
            let ctx = storage::resolve_context(home, journey.as_deref(), cwd)?;
            Ok(ctx.path.join(storage::README_FILE).display().to_string())
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

pub(crate) fn set_status(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    status: JourneyStatus,
) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    let previous_status = ctx.journey.status;
    let now = events::now_rfc3339()?;
    let attached = if status.is_open() && !previous_status.is_open() {
        storage::attach_journey_worktrees(home, &ctx.journey, &now)?
    } else {
        0
    };
    ctx.journey.status = status;
    if status.is_open() {
        ctx.journey.outcome = None;
    }
    storage::save_journey(&ctx.path, &ctx.journey)?;
    let detached = if !status.is_open() {
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

pub(crate) use worktree::{
    create_worktree, delete_worktree, link_repo, unlink_repo, WorktreeRequest,
};
