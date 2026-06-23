use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use console::style;

use crate::models::{IndexEntry, JourneyFile, JourneyStatus};
use crate::storage;

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

pub fn run_journey_list(
    default_filter: JourneyStatus,
    rows: &[IndexEntry],
    cwd: &Path,
) -> Result<()> {
    ensure_fzf()?;

    let default_query = default_filter.to_string();
    let input = candidate_lines(rows, Some(&default_query));
    let exe = shell_quote(&env::current_exe().context("failed to resolve current executable")?);
    let action_command = format!("{exe} __fzf-action-menu {{1}}");
    let preview_command = format!("{exe} __fzf-preview {{1}}");
    let reload_command = format!("{exe} __fzf-candidates --query={{q}}");
    let enter_bind = format!("enter:execute({action_command})+reload({reload_command})");
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

    let cwd_str = shell_quote(cwd);
    let new_journey_command = format!("{exe} __fzf-new-journey --cwd={cwd_str}");
    let new_bind = format!("ctrl-n:execute({new_journey_command})+reload({reload_command})");

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
        .arg(format!(
            "--header=Journey list | enter: actions | ctrl-n: new journey{git_hint} | ctrl-r: reload"
        ))
        .arg(format!("--delimiter={}", "\t"))
        .arg("--with-nth=2..")
        .arg("--preview-window=right:60%:wrap")
        .arg(format!("--preview={preview_command}"))
        .arg(format!("--bind={enter_bind}"))
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

pub fn candidate_lines(rows: &[IndexEntry], query: Option<&str>) -> String {
    rows.iter()
        .filter(|entry| matches_query(entry, query))
        .map(|entry| format!("{}\t{}", entry.id, sanitize_item(&entry.title)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn matches_query(entry: &IndexEntry, query: Option<&str>) -> bool {
    let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) else {
        return true;
    };
    let haystack = format!(
        "{} {} {} {} {}",
        entry.id,
        entry.title,
        entry.description.as_deref().unwrap_or(""),
        entry.status,
        entry.repos.join(" ")
    )
    .to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|term| haystack.contains(term))
}

pub fn preview_for_id(home: &Path, id: &str) -> Result<String> {
    storage::ensure_home(home)?;
    let journey_path = storage::journey_dir(home, id);
    let journey = storage::load_journey(&journey_path)
        .with_context(|| format!("failed to load Journey `{id}`"))?;
    let index = storage::load_index(home)?;
    let entry = index
        .journeys
        .iter()
        .find(|entry| entry.id == journey.id)
        .cloned()
        .unwrap_or_else(|| IndexEntry {
            id: journey.id.clone(),
            title: journey.title.clone(),
            description: journey.description.clone(),
            status: journey.status,
            updated: journey.created.clone(),
            repos: journey.repos.iter().map(|repo| repo.name.clone()).collect(),
        });
    Ok(build_preview(&journey, &entry, &journey_path))
}

pub fn pick_journey_action(journey_id: &str) -> Result<Option<String>> {
    ensure_fzf()?;

    let input = JOURNEY_ACTIONS
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
        .arg("--prompt=Action> ")
        .arg(format!("--header=Actions for {journey_id} | esc: back"))
        .arg(format!("--delimiter={}", "\t"))
        .arg("--with-nth=2..")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to start fzf action menu")?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open fzf action menu stdin"))?;
        stdin.write_all(input.as_bytes())?;
    }

    let output = child.wait_with_output()?;
    if output.status.success() {
        let selected = String::from_utf8(output.stdout).context("fzf output was not UTF-8")?;
        let action = selected
            .trim_end()
            .split_once('\t')
            .map(|(action, _)| action)
            .unwrap_or_else(|| selected.trim());
        if action.is_empty() {
            Ok(None)
        } else {
            Ok(Some(action.to_string()))
        }
    } else if matches!(output.status.code(), Some(1 | 130)) {
        Ok(None)
    } else {
        bail!("fzf action menu exited with status {}", output.status);
    }
}

pub fn fzf_notify(message: &str) -> Result<()> {
    ensure_fzf()?;

    let mut child = Command::new("fzf")
        .arg("--no-info")
        .arg("--border=rounded")
        .arg("--layout=reverse")
        .arg("--height=20%")
        .arg("--margin=5%,10%")
        .arg("--padding=1")
        .arg("--prompt=  ")
        .arg(format!("--header={message}"))
        .arg("--bind=enter:accept")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .context("failed to start fzf notification")?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open fzf stdin"))?;
        stdin.write_all(b"OK")?;
    }

    let _ = child.wait()?;
    Ok(())
}

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
            let default_path = repo_root.join(format!(".worktrees/{default_slug}"));
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

pub fn pick_repo_to_unlink(
    journey_id: &str,
    repos: &[crate::models::RepoRef],
) -> Result<Option<String>> {
    ensure_fzf()?;

    let input = repos
        .iter()
        .map(|r| format!("{}\t{}  ({})", r.name, r.name, r.worktree.display()))
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
        .arg("--prompt=Unlink> ")
        .arg(format!(
            "--header=Select repo to unlink from {journey_id} | esc: cancel"
        ))
        .arg(format!("--delimiter={}", "\t"))
        .arg("--with-nth=2..")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to start fzf repo picker")?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open fzf stdin"))?;
        stdin.write_all(input.as_bytes())?;
    }

    let output = child.wait_with_output()?;
    if matches!(output.status.code(), Some(1 | 130)) {
        return Ok(None);
    }
    if !output.status.success() {
        bail!("fzf repo picker exited with status {}", output.status);
    }

    let selected = String::from_utf8(output.stdout).context("fzf output was not UTF-8")?;
    let name = selected
        .trim_end()
        .split_once('\t')
        .map(|(key, _)| key)
        .unwrap_or_else(|| selected.trim());

    if name.is_empty() {
        Ok(None)
    } else {
        Ok(Some(name.to_string()))
    }
}

pub struct NewWorktreeInput {
    pub path: PathBuf,
    pub branch: String,
}

pub fn pick_new_worktree(journey_id: &str, repo_root: &Path) -> Result<Option<NewWorktreeInput>> {
    let slug = crate::storage::slugify(journey_id);

    let branch = fzf_prompt_text(
        "Branch>",
        &format!("New branch name (default: {slug}) | esc: cancel"),
        false,
    )?;
    let Some(branch_raw) = branch else {
        return Ok(None);
    };
    let branch = if branch_raw.is_empty() {
        slug.clone()
    } else {
        branch_raw
    };

    let default_path = repo_root.join(format!(".worktrees/{branch}"));
    let default_path_str = default_path.display().to_string();

    let path_str = fzf_prompt_text(
        "Path>",
        &format!("Worktree path (default: {default_path_str}) | esc: cancel"),
        false,
    )?;
    let Some(path_raw) = path_str else {
        return Ok(None);
    };
    let path = if path_raw.is_empty() {
        default_path
    } else {
        PathBuf::from(path_raw)
    };

    Ok(Some(NewWorktreeInput { path, branch }))
}

fn ensure_fzf() -> Result<()> {
    match Command::new("fzf").arg("--version").stdout(Stdio::null()).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => bail!(
            "fzf is required for interactive `journey list` but exited with status {status}; use `journey list --non-interactive` for table output"
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => bail!(
            "fzf is required for interactive `journey list`; install fzf or use `journey list --non-interactive`"
        ),
        Err(err) => Err(err).context("failed to check fzf availability"),
    }
}

fn build_preview(journey: &JourneyFile, entry: &IndexEntry, journey_path: &Path) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}", color(style(&journey.title).cyan().bold()));
    if let Some(description) = &journey.description {
        let _ = writeln!(out, "{} {}", label("description:"), description);
    }
    let _ = writeln!(out, "{} {}", label("id:"), journey.id);
    let _ = writeln!(
        out,
        "{} {}",
        label("status:"),
        styled_status(journey.status)
    );
    let _ = writeln!(out, "{} {}", label("updated:"), entry.updated);
    let _ = writeln!(
        out,
        "{} {}",
        label("path:"),
        color(style(journey_path.display()).dim())
    );
    out.push('\n');

    if journey.repos.is_empty() {
        let _ = writeln!(out, "{} none", label("repos:"));
    } else {
        let _ = writeln!(out, "{}", label("repos:"));
        for repo in &journey.repos {
            let _ = writeln!(
                out,
                "- {}  {}",
                color(style(&repo.name).cyan()),
                color(style(&repo.branch).dim())
            );
        }
    }
    out.push('\n');

    render_docs(&mut out, journey_path);
    out
}

fn render_docs(out: &mut String, journey_path: &Path) {
    let _ = writeln!(out, "{}", label("docs:"));
    let docs_dir = journey_path.join(storage::DOCS_DIR);
    let Ok(entries) = fs::read_dir(&docs_dir) else {
        let _ = writeln!(out, "- {}", color(style("none").dim()));
        return;
    };

    let mut docs = entries
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
        let _ = writeln!(out, "- {}", color(style("none").dim()));
    } else {
        for doc in docs.iter().take(8) {
            let _ = writeln!(out, "- {}", color(style(format!("docs/{doc}")).cyan()));
        }
        if docs.len() > 8 {
            let _ = writeln!(out, "- ... {} more", docs.len() - 8);
        }
    }
}

fn styled_status(status: JourneyStatus) -> String {
    match status {
        JourneyStatus::Active => color(style(status).green().bold()),
        JourneyStatus::Paused => color(style(status).yellow()),
        JourneyStatus::Archived => color(style(status).dim()),
        JourneyStatus::Abandoned => color(style(status).red()),
    }
}

fn label(value: &str) -> String {
    color(style(value).dim())
}

fn color<D: std::fmt::Display>(value: console::StyledObject<D>) -> String {
    value.force_styling(true).to_string()
}

fn sanitize_item(value: &str) -> String {
    value.replace(['\t', '\n', '\r'], " ")
}

fn shell_quote(path: &Path) -> String {
    let value = path.display().to_string();
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::JOURNEY_ACTIONS;

    #[test]
    fn journey_action_menu_opens_shell_first() {
        assert_eq!(JOURNEY_ACTIONS[0], ("shell", "cd journey"));
        assert_eq!(JOURNEY_ACTIONS[1], ("resume", "Resume"));
        assert_eq!(JOURNEY_ACTIONS[2], ("worktree", "New branch + worktree"));
    }
}
