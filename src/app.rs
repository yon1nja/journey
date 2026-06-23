use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use console::{measure_text_width, style, Term};

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

#[derive(Default)]
struct StarterDraft {
    title: Option<String>,
    description: Option<String>,
    journey_id: Option<String>,
    journey_path: Option<PathBuf>,
    worktree_mode: Option<String>,
    linked_worktrees: Vec<String>,
    messages: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StarterStep {
    Details,
    Worktrees,
    Create,
    Done,
}

fn start_journey_tui(home: &Path, cwd: &Path) -> Result<String> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("`journey` without a subcommand starts an interactive terminal UI; use `journey new <title>` in non-interactive contexts");
    }

    let discovered = git::discover_repo(cwd).ok();
    let mut draft = StarterDraft::default();

    let default_title = cwd
        .file_name()
        .map(|name| name.to_string_lossy().replace(['-', '_'], " "))
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "New Journey".to_string());

    render_starter_ui(cwd, discovered.as_ref(), &draft, StarterStep::Details)?;
    let title = prompt_required("Journey title", Some(&default_title))?;
    draft.title = Some(title.clone());

    render_starter_ui(cwd, discovered.as_ref(), &draft, StarterStep::Details)?;
    let description = clean_optional(prompt_optional(
        "Short description",
        "Why did you open this Journey?",
    )?);
    draft.description = description.clone();

    render_starter_ui(cwd, discovered.as_ref(), &draft, StarterStep::Create)?;
    let now = events::now_rfc3339()?;
    let mut ctx = storage::create_journey(home, &title, description, &now)?;
    draft.journey_id = Some(ctx.journey.id.clone());
    draft.journey_path = Some(ctx.path.clone());
    draft.messages.push(format!(
        "created Journey `{}` at {}",
        ctx.journey.id,
        ctx.path.display()
    ));

    if let Some(repo) = discovered.as_ref() {
        render_starter_ui(cwd, Some(repo), &draft, StarterStep::Worktrees)?;
        let choice = prompt_choice("Choose", &["1", "2", "3", "4"], "1")?;
        draft.worktree_mode = Some(worktree_choice_label(&choice).to_string());

        match choice.as_str() {
            "1" => {
                let name = prompt_repo_name(&repo.root)?;
                let linked = link_repo(
                    home,
                    cwd,
                    Some(&ctx.journey.id),
                    &repo.root,
                    Some(name.clone()),
                )?;
                draft.linked_worktrees.push(name);
                draft.messages.push(linked);
            }
            "2" => {
                render_starter_ui(cwd, Some(repo), &draft, StarterStep::Worktrees)?;
                let name =
                    create_and_link_worktree(home, cwd, &mut ctx, repo, &mut draft.messages)?;
                draft.linked_worktrees.push(name);
            }
            "3" => loop {
                render_starter_ui(cwd, Some(repo), &draft, StarterStep::Worktrees)?;
                let name =
                    create_and_link_worktree(home, cwd, &mut ctx, repo, &mut draft.messages)?;
                draft.linked_worktrees.push(name);
                if !prompt_yes_no("Create another worktree?", false)? {
                    break;
                }
            },
            "4" => {}
            _ => unreachable!("choice is validated"),
        }
    } else {
        draft.worktree_mode = Some("No git repo detected".to_string());
    }

    render_starter_ui(cwd, discovered.as_ref(), &draft, StarterStep::Done)?;
    println!("{}", ui_success("Done"));
    for message in &draft.messages {
        println!("  {} {message}", ui_dim("-"));
    }
    if let Some(path) = &draft.journey_path {
        println!("\n{} {}", ui_label("Journey folder:"), path.display());
    }
    Ok(String::new())
}

fn create_and_link_worktree(
    home: &Path,
    cwd: &Path,
    ctx: &mut JourneyContext,
    repo: &git::DiscoveredRepo,
    messages: &mut Vec<String>,
) -> Result<String> {
    let default_name = storage::slugify(&ctx.journey.title);
    let default_path = default_worktree_path(&repo.root, &default_name);
    let path = prompt_path("Worktree path", &default_path)?;
    let branch = prompt_required("Branch name", Some(&default_name))?;
    let create_branch = prompt_yes_no("Create a new branch for this worktree?", true)?;
    let default_repo_name = path.file_name().map(|name| name.to_string_lossy());
    let repo_name = validate_name(&prompt_required(
        "Journey repo name",
        default_repo_name.as_deref(),
    )?)?;
    if ctx.journey.repos.iter().any(|repo| repo.name == repo_name) {
        bail!("repo name `{repo_name}` is already linked; choose another name");
    }

    git::create_worktree(&repo.root, &path, &branch, create_branch)?;
    messages.push(format!(
        "created git worktree {} on branch `{}`",
        path.display(),
        branch
    ));
    let linked_repo_name = repo_name.clone();
    messages.push(link_repo(
        home,
        cwd,
        Some(&ctx.journey.id),
        &path,
        Some(repo_name),
    )?);
    ctx.path = storage::journey_dir(home, &ctx.journey.id);
    ctx.journey = storage::load_journey(&ctx.path)?;
    Ok(linked_repo_name)
}

fn default_worktree_path(repo_root: &Path, journey_slug: &str) -> PathBuf {
    let repo_parent = repo_root.parent().unwrap_or(repo_root);
    let repo_name = repo_root
        .file_name()
        .map(|name| storage::slugify(&name.to_string_lossy()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "repo".to_string());
    let leaf = if repo_name == journey_slug {
        format!("{journey_slug}-worktree")
    } else {
        format!("{repo_name}-{journey_slug}")
    };
    repo_parent.join(leaf)
}

fn prompt_repo_name(repo_root: &Path) -> Result<String> {
    let default = repo_root
        .file_name()
        .map(|name| name.to_string_lossy())
        .filter(|name| !name.trim().is_empty());
    prompt_required("Journey repo name", default.as_deref())
}

fn render_starter_ui(
    cwd: &Path,
    repo: Option<&git::DiscoveredRepo>,
    draft: &StarterDraft,
    step: StarterStep,
) -> Result<()> {
    clear_screen()?;
    let width = terminal_width();
    if width >= 72 {
        render_starter_split(cwd, repo, draft, step, width);
    } else {
        render_starter_single(cwd, repo, draft, step, width);
    }
    io::stdout().flush()?;
    Ok(())
}

fn render_starter_split(
    cwd: &Path,
    repo: Option<&git::DiscoveredRepo>,
    draft: &StarterDraft,
    step: StarterStep,
    width: usize,
) {
    let left_width = if width >= 104 { 32 } else { 28 };
    let right_width = width - left_width - 7;
    let left = starter_left_lines(repo, draft, step);
    let right = starter_preview_lines(cwd, repo, draft, right_width);
    let rows = left.len().max(right.len());

    println!("{}", ui_border(width));
    println!(
        "| {} |",
        pad_ansi(
            &format!(
                "{}  {}",
                ui_active("Journey starter"),
                ui_dim("local effort from the current folder")
            ),
            width - 4
        )
    );
    println!("{}", ui_split_border(left_width, right_width));
    println!(
        "| {} | {} |",
        pad_ansi(&ui_dim("Steps"), left_width),
        pad_ansi(&ui_dim("Preview"), right_width)
    );
    println!("{}", ui_split_border(left_width, right_width));
    for idx in 0..rows {
        let left_line = left.get(idx).map(String::as_str).unwrap_or("");
        let right_line = right.get(idx).map(String::as_str).unwrap_or("");
        println!(
            "| {} | {} |",
            pad_ansi(left_line, left_width),
            pad_ansi(right_line, right_width)
        );
    }
    println!("{}", ui_split_border(left_width, right_width));
    println!(
        "| {} |",
        pad_ansi(
            &format!(
                "{} {}",
                ui_label("Enter"),
                "accepts defaults; empty description is allowed"
            ),
            width - 4
        )
    );
    println!("{}", ui_border(width));
    println!();
}

fn render_starter_single(
    cwd: &Path,
    repo: Option<&git::DiscoveredRepo>,
    draft: &StarterDraft,
    step: StarterStep,
    width: usize,
) {
    println!("{}", ui_border(width));
    println!("| {} |", pad_ansi(&ui_active("Journey starter"), width - 4));
    println!("{}", ui_border(width));
    for line in starter_left_lines(repo, draft, step) {
        println!("| {} |", pad_ansi(&line, width - 4));
    }
    println!("{}", ui_border(width));
    for line in starter_preview_lines(cwd, repo, draft, width - 4) {
        println!("| {} |", pad_ansi(&line, width - 4));
    }
    println!("{}", ui_border(width));
    println!();
}

fn starter_left_lines(
    repo: Option<&git::DiscoveredRepo>,
    draft: &StarterDraft,
    step: StarterStep,
) -> Vec<String> {
    let mut lines = vec![
        starter_step_line(step, StarterStep::Details, "Details"),
        starter_step_line(step, StarterStep::Worktrees, "Worktrees"),
        starter_step_line(step, StarterStep::Create, "Create"),
        starter_step_line(step, StarterStep::Done, "Done"),
    ];

    lines.push(String::new());
    lines.push(format!(
        "{} {}",
        ui_label("status:"),
        starter_step_status(step)
    ));
    if let Some(mode) = &draft.worktree_mode {
        lines.push(format!("{} {}", ui_label("mode:"), mode));
    }

    if repo.is_some() && matches!(step, StarterStep::Worktrees) {
        lines.push(String::new());
        lines.push(ui_dim("Worktree action"));
        lines.push(format!("{} Attach current worktree", ui_active("1")));
        lines.push(format!("{} Create one worktree", ui_active("2")));
        lines.push(format!("{} Create multiple worktrees", ui_active("3")));
        lines.push(format!("{} Skip for now", ui_active("4")));
    }

    lines
}

fn starter_preview_lines(
    cwd: &Path,
    repo: Option<&git::DiscoveredRepo>,
    draft: &StarterDraft,
    width: usize,
) -> Vec<String> {
    let value_width = width.saturating_sub(14).max(12);
    let mut lines = vec![
        ui_active("Current context"),
        field_line(
            "folder:",
            &compact_text(&cwd.display().to_string(), value_width),
        ),
    ];

    match repo {
        Some(repo) => {
            lines.push(field_line(
                "git root:",
                &compact_text(&repo.root.display().to_string(), value_width),
            ));
            lines.push(field_line(
                "branch:",
                &compact_text(&repo.branch, value_width),
            ));
        }
        None => lines.push(field_line("git repo:", "none detected")),
    }

    lines.push(String::new());
    lines.push(ui_active("Draft"));
    lines.push(field_line(
        "title:",
        &optional_compact(draft.title.as_deref(), value_width),
    ));
    lines.push(field_line(
        "desc:",
        &optional_compact(draft.description.as_deref(), value_width),
    ));

    if let Some(id) = &draft.journey_id {
        lines.push(field_line("id:", id));
    }
    if let Some(path) = &draft.journey_path {
        lines.push(field_line(
            "path:",
            &compact_text(&path.display().to_string(), value_width),
        ));
    }
    if !draft.linked_worktrees.is_empty() {
        lines.push(field_line(
            "linked:",
            &compact_text(&draft.linked_worktrees.join(", "), value_width),
        ));
    }

    if !draft.messages.is_empty() {
        lines.push(String::new());
        lines.push(ui_active("Activity"));
        for message in draft.messages.iter().rev().take(3).rev() {
            lines.push(format!(
                "{} {}",
                ui_dim("-"),
                compact_text(message, width.saturating_sub(4).max(12))
            ));
        }
    }

    lines
}

fn starter_step_line(current: StarterStep, step: StarterStep, label: &str) -> String {
    if current == step {
        format!("{} {}", ui_active(">"), ui_active(label))
    } else {
        format!("{} {}", ui_dim(" "), label)
    }
}

fn starter_step_status(step: StarterStep) -> String {
    match step {
        StarterStep::Details => ui_active("details"),
        StarterStep::Worktrees => ui_active("worktrees"),
        StarterStep::Create => ui_active("creating"),
        StarterStep::Done => ui_success("done"),
    }
}

fn worktree_choice_label(choice: &str) -> &'static str {
    match choice {
        "1" => "Attach current worktree",
        "2" => "Create one worktree",
        "3" => "Create multiple worktrees",
        "4" => "Skip worktrees",
        _ => "Unknown",
    }
}

fn field_line(label: &str, value: &str) -> String {
    let value = if value == "not set" || value == "none detected" {
        ui_dim(value)
    } else {
        value.to_string()
    };
    format!("{} {}", ui_label(label), value)
}

fn optional_compact(value: Option<&str>, width: usize) -> String {
    value
        .map(|value| compact_text(value, width))
        .unwrap_or_else(|| "not set".to_string())
}

fn terminal_width() -> usize {
    let (_, columns) = Term::stdout().size();
    usize::from(columns).clamp(60, 120)
}

fn ui_border(width: usize) -> String {
    ui_dim(&format!("+{}+", "-".repeat(width.saturating_sub(2))))
}

fn ui_split_border(left_width: usize, right_width: usize) -> String {
    ui_dim(&format!(
        "+{}+{}+",
        "-".repeat(left_width + 2),
        "-".repeat(right_width + 2)
    ))
}

fn pad_ansi(value: &str, width: usize) -> String {
    let visible = measure_text_width(value);
    if visible >= width {
        value.to_string()
    } else {
        format!("{value}{}", " ".repeat(width - visible))
    }
}

fn compact_text(value: &str, max_width: usize) -> String {
    if measure_text_width(value) <= max_width {
        return value.to_string();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }

    let keep = max_width - 3;
    let left = keep / 2;
    let right = keep - left;
    let prefix = value.chars().take(left).collect::<String>();
    let suffix = value
        .chars()
        .rev()
        .take(right)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{prefix}...{suffix}")
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

fn prompt_required(label: &str, default: Option<&str>) -> Result<String> {
    loop {
        let value = prompt_line(label, default)?;
        if !value.trim().is_empty() {
            return Ok(value.trim().to_string());
        }
        println!("{label} cannot be empty");
    }
}

fn prompt_optional(label: &str, hint: &str) -> Result<Option<String>> {
    println!("{}", ui_dim(hint));
    let value = prompt_line(label, None)?;
    Ok(if value.trim().is_empty() {
        None
    } else {
        Some(value.trim().to_string())
    })
}

fn prompt_choice(label: &str, allowed: &[&str], default: &str) -> Result<String> {
    loop {
        let value = prompt_line(label, Some(default))?;
        let value = value.trim();
        if allowed.contains(&value) {
            return Ok(value.to_string());
        }
        println!("Choose one of: {}", allowed.join(", "));
    }
}

fn prompt_yes_no(label: &str, default: bool) -> Result<bool> {
    let default_label = if default { "Y/n" } else { "y/N" };
    loop {
        let value = prompt_line(&format!("{label} [{default_label}]"), None)?;
        let value = value.trim().to_ascii_lowercase();
        if value.is_empty() {
            return Ok(default);
        }
        match value.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("Enter yes or no"),
        }
    }
}

fn prompt_path(label: &str, default: &Path) -> Result<PathBuf> {
    let value = prompt_required(label, Some(&default.display().to_string()))?;
    let path = PathBuf::from(value);
    Ok(if path.is_absolute() {
        path
    } else {
        env::current_dir()?.join(path)
    })
}

fn prompt_line(label: &str, default: Option<&str>) -> Result<String> {
    print!("{} ", ui_active("Journey>"));
    match default {
        Some(default) => print!("{label} [{default}]: "),
        None => print!("{label}: "),
    }
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim_end_matches(['\r', '\n']).to_string();
    if value.trim().is_empty() {
        Ok(default.unwrap_or("").to_string())
    } else {
        Ok(value)
    }
}

fn clear_screen() -> Result<()> {
    print!("\x1b[2J\x1b[H");
    io::stdout().flush()?;
    Ok(())
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
