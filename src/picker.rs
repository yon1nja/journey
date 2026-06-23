use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use console::style;

use crate::models::{IndexEntry, JourneyFile, JourneyStatus};
use crate::storage;

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
