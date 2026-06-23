use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::cli::{join_words, Cli, Commands, DocCommands};
use crate::events;
use crate::git;
use crate::models::{CheckpointRepo, EventKind, EventRecord, JourneyStatus, RepoRef};
use crate::projection;
use crate::storage::{self, JourneyContext};

pub fn run(cli: Cli) -> Result<String> {
    let home = storage::journey_home()?;
    let cwd = env::current_dir().context("failed to read current directory")?;

    match cli.command {
        Commands::New(args) => new_journey(&home, &join_words(&args.text)),
        Commands::Link {
            repo_path,
            name,
            journey,
        } => link_repo(&home, &cwd, journey.as_deref(), &repo_path, name),
        Commands::Checkpoint { message, journey } => {
            checkpoint(&home, &cwd, journey.as_deref(), message)
        }
        Commands::Resume { id, apply } => resume(&home, &cwd, id.as_deref(), apply),
        Commands::List { status } => list_journeys(&home, status.unwrap_or(JourneyStatus::Active)),
        Commands::Status { id } => status(&home, &cwd, id.as_deref()),
        Commands::Note(args) => append_simple_event(
            &home,
            &cwd,
            args.journey.as_deref(),
            EventKind::Note {
                text: join_words(&args.text),
            },
            "recorded note",
        ),
        Commands::Decide(args) => decide(
            &home,
            &cwd,
            args.journey.as_deref(),
            args.text,
            args.because,
        ),
        Commands::Ask(args) => ask(&home, &cwd, args.journey.as_deref(), args.text),
        Commands::Resolve(args) => resolve_question(
            &home,
            &cwd,
            args.journey.as_deref(),
            args.qid,
            join_words(&args.answer),
        ),
        Commands::Next(args) => next_actions(&home, &cwd, args.journey.as_deref(), args.items),
        Commands::Doc { command } => doc_command(&home, &cwd, command),
        Commands::Pause(args) => set_status(&home, &cwd, args.id.as_deref(), JourneyStatus::Paused),
        Commands::Archive(args) => {
            set_status(&home, &cwd, args.id.as_deref(), JourneyStatus::Archived)
        }
        Commands::Abandon(args) => {
            set_status(&home, &cwd, args.id.as_deref(), JourneyStatus::Abandoned)
        }
    }
}

fn new_journey(home: &Path, title: &str) -> Result<String> {
    let now = events::now_rfc3339()?;
    let ctx = storage::create_journey(home, title, &now)?;
    let events = events::read_events(&ctx.path)?;
    projection::write_now(&ctx.path, &ctx.journey, &events)?;
    Ok(format!(
        "created Journey `{}` at {}",
        ctx.journey.id,
        ctx.path.display()
    ))
}

fn list_journeys(home: &Path, status: JourneyStatus) -> Result<String> {
    storage::ensure_home(home)?;
    let index = storage::load_index(home)?;
    let mut rows = index
        .journeys
        .iter()
        .filter(|entry| entry.status == status)
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));

    if rows.is_empty() {
        return Ok(format!("no {status} Journeys"));
    }

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

fn status(home: &Path, cwd: &Path, id: Option<&str>) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let events = events::read_events(&ctx.path)?;
    let latest_checkpoint = latest_checkpoint(&events);

    let mut out = String::new();
    out.push_str(&format!("{}\n", ctx.journey.title));
    out.push_str(&format!("id: {}\n", ctx.journey.id));
    out.push_str(&format!("status: {}\n", ctx.journey.status));
    out.push_str(&format!("path: {}\n", ctx.path.display()));
    out.push_str(&format!("repos: {}\n", ctx.journey.repos.len()));
    out.push_str(&format!("events: {}\n", events.len()));
    if let Some(event) = latest_checkpoint {
        out.push_str(&format!(
            "latest checkpoint: #{} at {}\n",
            event.seq, event.ts
        ));
    } else {
        out.push_str("latest checkpoint: none\n");
    }
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

    let repo = RepoRef {
        name: repo_name.clone(),
        root: discovered.root.clone(),
        worktree: discovered.root,
        branch: discovered.branch.clone(),
    };
    ctx.journey.repos.push(repo.clone());
    storage::save_journey(&ctx.path, &ctx.journey)?;
    storage::sync_worktree_link(&ctx.path, &repo)?;

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

fn checkpoint(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    message: Option<String>,
) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let existing_events = events::read_events(&ctx.path)?;
    let seq = existing_events
        .iter()
        .map(|event| event.seq)
        .max()
        .unwrap_or(0)
        + 1;

    let mut repos = Vec::new();
    for repo in &ctx.journey.repos {
        let state = git::collect_state(&repo.worktree).with_context(|| {
            format!(
                "failed to collect git state for `{}` at {}",
                repo.name,
                repo.worktree.display()
            )
        })?;
        let dirty_snapshot_ref = if state.tracked_dirty {
            git::create_dirty_snapshot(&repo.worktree, &ctx.journey.id, seq, &repo.name)?
        } else {
            None
        };
        repos.push(CheckpointRepo {
            name: repo.name.clone(),
            head: state.head,
            branch: state.branch,
            upstream: state.upstream,
            ahead: state.ahead,
            behind: state.behind,
            tracked_dirty: state.tracked_dirty,
            dirty_snapshot_ref,
            untracked_files: state.untracked_files,
        });
    }

    let record = events::append_event(
        &ctx.path,
        EventKind::Checkpoint {
            message,
            repos: repos.clone(),
        },
    )?;
    finish_mutation(&ctx, Some(record.ts.clone()))?;

    let dirty = repos.iter().filter(|repo| repo.tracked_dirty).count();
    let untracked: usize = repos.iter().map(|repo| repo.untracked_files.len()).sum();
    Ok(format!(
        "checkpoint #{} recorded for Journey `{}` ({} repos, {} dirty, {} untracked files)",
        record.seq,
        ctx.journey.id,
        repos.len(),
        dirty,
        untracked
    ))
}

fn resume(home: &Path, cwd: &Path, id: Option<&str>, apply: bool) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let events = events::read_events(&ctx.path)?;
    let checkpoint = latest_checkpoint(&events);
    let latest_by_repo = latest_checkpoint_repos(checkpoint);

    let mut out = String::new();
    out.push_str(&format!(
        "Journey `{}` - {}\n",
        ctx.journey.id, ctx.journey.title
    ));
    out.push_str(&format!("path: {}\n\n", ctx.path.display()));

    for repo in &ctx.journey.repos {
        out.push_str(&format!("## {}\n", repo.name));
        if !repo.worktree.exists() {
            match git::ensure_worktree(&repo.root, &repo.worktree, &repo.branch) {
                Ok(()) => out.push_str(&format!(
                    "- created missing worktree at {}\n",
                    repo.worktree.display()
                )),
                Err(err) => {
                    out.push_str(&format!("- could not create missing worktree: {err}\n\n"));
                    continue;
                }
            }
        }

        let state = match git::collect_state(&repo.worktree) {
            Ok(state) => state,
            Err(err) => {
                out.push_str(&format!("- could not read git state: {err}\n\n"));
                continue;
            }
        };
        out.push_str(&format!("- live branch: `{}`\n", state.branch));
        out.push_str(&format!("- live head: `{}`\n", short_sha(&state.head)));

        if let Some(saved) = latest_by_repo.get(&repo.name) {
            if saved.head != state.head {
                out.push_str(&format!(
                    "- drift: checkpoint head `{}` differs from live `{}`\n",
                    short_sha(&saved.head),
                    short_sha(&state.head)
                ));
            }
            if saved.branch != state.branch {
                out.push_str(&format!(
                    "- drift: checkpoint branch `{}` differs from live `{}`\n",
                    saved.branch, state.branch
                ));
            }
            if let Some(snapshot_ref) = &saved.dirty_snapshot_ref {
                match git::diff_stat(&repo.worktree, &saved.head, snapshot_ref) {
                    Ok(stat) => {
                        out.push_str("- dirty snapshot stat:\n");
                        out.push_str("```text\n");
                        out.push_str(&stat);
                        out.push_str("\n```\n");
                    }
                    Err(err) => {
                        out.push_str(&format!("- dirty snapshot stat unavailable: {err}\n"));
                    }
                }
                if apply {
                    git::apply_snapshot(&repo.worktree, snapshot_ref)?;
                    out.push_str("- applied dirty snapshot\n");
                }
            }
            if !saved.untracked_files.is_empty() {
                out.push_str(&format!(
                    "- warning: {} untracked files were recorded at checkpoint time but were not snapshotted\n",
                    saved.untracked_files.len()
                ));
            }
        } else {
            out.push_str("- no checkpoint data for this repo\n");
        }
        out.push('\n');
    }

    projection::write_now(&ctx.path, &ctx.journey, &events)?;
    out.push_str("## NOW\n\n");
    out.push_str(&projection::render_now(&ctx.path, &ctx.journey, &events));
    Ok(out.trim_end().to_string())
}

fn append_simple_event(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    kind: EventKind,
    message: &str,
) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let record = events::append_event(&ctx.path, kind)?;
    finish_mutation(&ctx, Some(record.ts))?;
    Ok(format!("{message} in Journey `{}`", ctx.journey.id))
}

fn decide(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    text: Vec<String>,
    because: Option<String>,
) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let current = events::read_events(&ctx.path)?;
    let did = events::next_decision_id(&current);
    let record = events::append_event(
        &ctx.path,
        EventKind::Decision {
            did: did.clone(),
            text: join_words(&text),
            because,
        },
    )?;
    finish_mutation(&ctx, Some(record.ts))?;
    Ok(format!(
        "recorded decision `{did}` in Journey `{}`",
        ctx.journey.id
    ))
}

fn ask(home: &Path, cwd: &Path, id: Option<&str>, text: Vec<String>) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let current = events::read_events(&ctx.path)?;
    let qid = events::next_question_id(&current);
    let record = events::append_event(
        &ctx.path,
        EventKind::QuestionOpen {
            qid: qid.clone(),
            text: join_words(&text),
        },
    )?;
    finish_mutation(&ctx, Some(record.ts))?;
    Ok(format!(
        "opened question `{qid}` in Journey `{}`",
        ctx.journey.id
    ))
}

fn resolve_question(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    qid: String,
    answer: String,
) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let record = events::append_event(
        &ctx.path,
        EventKind::QuestionResolve {
            qid: qid.clone(),
            answer,
        },
    )?;
    finish_mutation(&ctx, Some(record.ts))?;
    Ok(format!(
        "resolved question `{qid}` in Journey `{}`",
        ctx.journey.id
    ))
}

fn next_actions(home: &Path, cwd: &Path, id: Option<&str>, items: Vec<String>) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let record = events::append_event(&ctx.path, EventKind::NextActions { items })?;
    finish_mutation(&ctx, Some(record.ts))?;
    Ok(format!(
        "updated next actions in Journey `{}`",
        ctx.journey.id
    ))
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
            let events = events::read_events(&ctx.path)?;
            projection::write_now(&ctx.path, &ctx.journey, &events)?;
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

fn set_status(home: &Path, cwd: &Path, id: Option<&str>, status: JourneyStatus) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    ctx.journey.status = status;
    storage::save_journey(&ctx.path, &ctx.journey)?;
    let record = events::append_event(&ctx.path, EventKind::StatusChange { status })?;
    finish_mutation(&ctx, Some(record.ts))?;
    Ok(format!("Journey `{}` is now {}", ctx.journey.id, status))
}

fn finish_mutation(ctx: &JourneyContext, updated: Option<String>) -> Result<()> {
    let updated = match updated {
        Some(updated) => updated,
        None => events::now_rfc3339()?,
    };
    storage::update_index_entry(&ctx.home, &ctx.journey, &updated)?;
    let events = events::read_events(&ctx.path)?;
    projection::write_now(&ctx.path, &ctx.journey, &events)
}

fn latest_checkpoint(events: &[EventRecord]) -> Option<&EventRecord> {
    events
        .iter()
        .rev()
        .find(|event| matches!(event.kind, EventKind::Checkpoint { .. }))
}

fn latest_checkpoint_repos(event: Option<&EventRecord>) -> HashMap<String, CheckpointRepo> {
    let Some(event) = event else {
        return HashMap::new();
    };
    let EventKind::Checkpoint { repos, .. } = &event.kind else {
        return HashMap::new();
    };
    repos
        .iter()
        .cloned()
        .map(|repo| (repo.name.clone(), repo))
        .collect()
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

fn short_sha(value: &str) -> String {
    value.chars().take(12).collect()
}
