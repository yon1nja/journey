//! Read-only views of Journeys, as plain text or JSON for agents.

use std::path::PathBuf;

use serde::Serialize;

use super::*;

#[derive(Debug, Serialize)]
struct JourneySummary {
    id: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    status: JourneyStatus,
    updated: String,
    path: PathBuf,
    repos: Vec<String>,
}

#[derive(Debug, Serialize)]
struct JourneyDetail {
    id: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    status: JourneyStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
    created: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated: Option<String>,
    path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    readme: Option<PathBuf>,
    docs: Vec<String>,
    repos: Vec<RepoRef>,
    events: usize,
}

#[derive(Debug, Serialize)]
struct DocEntry {
    name: String,
    path: PathBuf,
}

pub(crate) struct ListFilter<'a> {
    pub status: Option<JourneyStatus>,
    pub all: bool,
    pub search: Option<&'a str>,
}

pub(crate) fn new_journey(
    home: &Path,
    title: &str,
    description: Option<String>,
    json: bool,
) -> Result<String> {
    let now = events::now_rfc3339()?;
    let ctx = storage::create_journey(home, title, clean_optional(description), &now)?;
    if json {
        return to_json(&detail(&ctx)?);
    }
    Ok(format!(
        "created Journey `{}` at {}",
        ctx.journey.id,
        ctx.path.display()
    ))
}

pub(crate) fn list_journeys(home: &Path, filter: &ListFilter<'_>, json: bool) -> Result<String> {
    storage::ensure_home(home)?;
    let index = storage::load_index(home)?;
    let query = filter.search.map(str::to_lowercase);
    // A search looks across every status unless one is given.
    let any_status = filter.all || (filter.search.is_some() && filter.status.is_none());
    let status = filter.status.unwrap_or(JourneyStatus::Active);

    let mut rows = index
        .journeys
        .into_iter()
        .filter(|entry| any_status || entry.status == status)
        .filter(|entry| query.as_deref().is_none_or(|query| matches(entry, query)))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));

    if json {
        let rows = rows
            .into_iter()
            .map(|entry| JourneySummary {
                path: storage::journey_dir(home, &entry.id),
                id: entry.id,
                title: entry.title,
                description: entry.description,
                status: entry.status,
                updated: entry.updated,
                repos: entry.repos,
            })
            .collect::<Vec<_>>();
        return to_json(&rows);
    }

    if rows.is_empty() {
        return Ok(match (&filter.search, any_status) {
            (Some(search), _) => format!("no Journeys match `{search}`"),
            (None, true) => "no Journeys".to_string(),
            (None, false) => format!("no {status} Journeys"),
        });
    }
    Ok(render_journey_table(&rows))
}

fn matches(entry: &IndexEntry, query: &str) -> bool {
    entry.id.to_lowercase().contains(query)
        || entry.title.to_lowercase().contains(query)
        || entry
            .description
            .as_deref()
            .is_some_and(|description| description.to_lowercase().contains(query))
        || entry
            .repos
            .iter()
            .any(|repo| repo.to_lowercase().contains(query))
}

fn render_journey_table(rows: &[IndexEntry]) -> String {
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
    out.trim_end().to_string()
}

pub(crate) fn status(home: &Path, cwd: &Path, id: Option<&str>, json: bool) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let detail = detail(&ctx)?;
    if json {
        return to_json(&detail);
    }

    let mut out = String::new();
    out.push_str(&format!("{}\n", detail.title));
    if let Some(description) = &detail.description {
        out.push_str(&format!("description: {description}\n"));
    }
    out.push_str(&format!("id: {}\n", detail.id));
    out.push_str(&format!("status: {}\n", detail.status));
    if let Some(outcome) = &detail.outcome {
        out.push_str(&format!("outcome: {outcome}\n"));
    }
    out.push_str(&format!("path: {}\n", detail.path.display()));
    match &detail.readme {
        Some(readme) => out.push_str(&format!("readme: {}\n", readme.display())),
        None => out.push_str("readme: none\n"),
    }
    out.push_str(&format!("docs: {}\n", detail.docs.len()));
    for doc in &detail.docs {
        out.push_str(&format!("  {doc}\n"));
    }
    out.push_str(&format!("repos: {}\n", detail.repos.len()));
    for repo in &detail.repos {
        out.push_str(&format!(
            "  {}\t{}\t{}\n",
            repo.name,
            repo.branch,
            repo.worktree.display()
        ));
    }
    out.push_str(&format!("events: {}\n", detail.events));
    Ok(out.trim_end().to_string())
}

fn detail(ctx: &JourneyContext) -> Result<JourneyDetail> {
    let updated = storage::load_index(&ctx.home)?
        .journeys
        .into_iter()
        .find(|entry| entry.id == ctx.journey.id)
        .map(|entry| entry.updated);
    let readme = ctx.path.join(storage::README_FILE);
    let journey = ctx.journey.clone();
    Ok(JourneyDetail {
        id: journey.id,
        title: journey.title,
        description: journey.description,
        status: journey.status,
        outcome: journey.outcome,
        created: journey.created,
        updated,
        path: ctx.path.clone(),
        readme: readme.exists().then_some(readme),
        docs: doc_names(&ctx.path)?,
        repos: journey.repos,
        events: events::read_events(&ctx.path)?.len(),
    })
}

pub(crate) fn list_docs(home: &Path, cwd: &Path, id: Option<&str>, json: bool) -> Result<String> {
    let ctx = storage::resolve_context(home, id, cwd)?;
    let docs = doc_names(&ctx.path)?;
    if json {
        let docs = docs
            .into_iter()
            .map(|name| DocEntry {
                path: ctx.path.join(storage::DOCS_DIR).join(&name),
                name,
            })
            .collect::<Vec<_>>();
        return to_json(&docs);
    }
    if docs.is_empty() {
        Ok("no docs".to_string())
    } else {
        Ok(docs.join("\n"))
    }
}

fn doc_names(journey_path: &Path) -> Result<Vec<String>> {
    let docs_dir = journey_path.join(storage::DOCS_DIR);
    if !docs_dir.exists() {
        return Ok(Vec::new());
    }
    let mut docs = fs::read_dir(&docs_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter_map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .collect::<Vec<_>>();
    docs.sort();
    Ok(docs)
}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)?)
}
