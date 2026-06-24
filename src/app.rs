use std::env;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::cli::{join_words, Cli, Commands, DocCommands, ReadmeCommands};
use crate::events;
use crate::git;
use crate::models::{EventKind, IndexEntry, JourneyStatus, RepoRef};
use crate::storage::{self, JourneyContext};
use crate::tui;

const SHELL_INTEGRATION_ENV: &str = "JOURNEY_SHELL_INTEGRATION";
pub(crate) const SHELL_CD_PREFIX: &str = "__journey_cd__\t";

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
        Some(Commands::Status { id }) => status(&home, &cwd, id.as_deref()),
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
    }
}

pub(crate) fn new_journey(home: &Path, title: &str, description: Option<String>) -> Result<String> {
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

    if !non_interactive
        && (io::stdout().is_terminal()
            || (shell_integration_active() && io::stderr().is_terminal()))
    {
        return Ok(tui::run_journey_app(home, cwd, default_filter)?.unwrap_or_default());
    }

    if rows.is_empty() {
        return Ok("no Journeys".to_string());
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

pub(crate) fn status(home: &Path, cwd: &Path, id: Option<&str>) -> Result<String> {
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

pub(crate) fn link_repo(
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

pub(crate) fn unlink_repo(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    repo_name: &str,
) -> Result<String> {
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

pub(crate) fn delete_worktree(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    repo_name: &str,
) -> Result<String> {
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

    let repo = ctx.journey.repos[position].clone();
    git::remove_worktree(&repo.root, &repo.worktree)?;

    ctx.journey.repos.remove(position);
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
        "deleted worktree {} and unlinked `{}` from Journey `{}`",
        repo.worktree.display(),
        repo.name,
        ctx.journey.id
    ))
}

pub(crate) fn done(home: &Path, cwd: &Path, id: Option<&str>) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    let repos = ctx.journey.repos.clone();
    for repo in &repos {
        git::ensure_worktree_removable(&repo.root, &repo.worktree)?;
    }

    let mut removed = 0;
    for repo in &repos {
        git::remove_worktree(&repo.root, &repo.worktree)?;
        storage::remove_worktree_link(&ctx.path, &repo.name)?;
        removed += 1;
    }

    ctx.journey.status = JourneyStatus::Archived;
    storage::save_journey(&ctx.path, &ctx.journey)?;
    storage::detach_journey_worktrees(home, &ctx.journey.id)?;
    let record = events::append_event(
        &ctx.path,
        EventKind::StatusChange {
            status: JourneyStatus::Archived,
        },
    )?;
    finish_mutation(&ctx, Some(record.ts))?;

    Ok(format!(
        "Journey `{}` is done: archived and removed {} worktrees",
        ctx.journey.id, removed
    ))
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

pub(crate) fn shell_quote(path: &Path) -> String {
    let value = path.display().to_string();
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shell_quote_value(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn shell_integration_active() -> bool {
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
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;

    fn git_command(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("failed to run git");

        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8(output.stdout).expect("git stdout was not UTF-8")
    }

    fn init_repo(root: &Path) {
        fs::create_dir_all(root).unwrap();
        git_command(root, &["init"]);
        git_command(root, &["config", "user.email", "journey@example.com"]);
        git_command(root, &["config", "user.name", "Journey Test"]);
        fs::write(root.join("README.md"), "initial\n").unwrap();
        git_command(root, &["add", "README.md"]);
        git_command(root, &["commit", "-m", "initial"]);
    }

    #[test]
    fn delete_worktree_removes_git_worktree_and_unlinks_repo() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("journey-home");
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("feature-worktree");
        init_repo(&repo);
        git_command(&repo, &["branch", "feature"]);
        git::create_worktree(&repo, &worktree, "feature", false).unwrap();

        new_journey(&home, "Delete Worktree", None).unwrap();
        link_repo(
            &home,
            &repo,
            Some("delete-worktree"),
            &worktree,
            Some("feature".to_string()),
        )
        .unwrap();

        let deleted = delete_worktree(&home, &repo, Some("delete-worktree"), "feature").unwrap();

        assert!(deleted.contains("deleted worktree"));
        assert!(!worktree.exists());
        let journey =
            storage::load_journey(&storage::journey_dir(&home, "delete-worktree")).unwrap();
        assert!(journey.repos.is_empty());
        assert!(storage::load_worktree_index(&home)
            .unwrap()
            .attachments
            .is_empty());
        let journal = fs::read_to_string(
            storage::journey_dir(&home, "delete-worktree").join(storage::JOURNAL_FILE),
        )
        .unwrap();
        assert!(journal.contains("\"type\":\"unlink_repo\""));
    }

    #[test]
    fn done_archives_and_removes_worktrees_while_keeping_context() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("journey-home");
        let repo = temp.path().join("repo");
        let first = temp.path().join("first-worktree");
        let second = temp.path().join("second-worktree");
        init_repo(&repo);
        git_command(&repo, &["branch", "first"]);
        git_command(&repo, &["branch", "second"]);
        git::create_worktree(&repo, &first, "first", false).unwrap();
        git::create_worktree(&repo, &second, "second", false).unwrap();

        new_journey(&home, "Done Journey", None).unwrap();
        link_repo(
            &home,
            &repo,
            Some("done-journey"),
            &first,
            Some("first".to_string()),
        )
        .unwrap();
        link_repo(
            &home,
            &repo,
            Some("done-journey"),
            &second,
            Some("second".to_string()),
        )
        .unwrap();

        let result = done(&home, &repo, Some("done-journey")).unwrap();

        assert!(result.contains("archived and removed 2 worktrees"));
        assert!(!first.exists());
        assert!(!second.exists());
        let journey_dir = storage::journey_dir(&home, "done-journey");
        let journey = storage::load_journey(&journey_dir).unwrap();
        assert_eq!(journey.status, JourneyStatus::Archived);
        assert_eq!(journey.repos.len(), 2);
        assert!(journey_dir.join(storage::JOURNAL_FILE).exists());
        assert!(storage::load_worktree_index(&home)
            .unwrap()
            .attachments
            .is_empty());
        assert!(!journey_dir
            .join(storage::WORKTREES_DIR)
            .join("first")
            .exists());
        assert!(!journey_dir
            .join(storage::WORKTREES_DIR)
            .join("second")
            .exists());
    }
}
