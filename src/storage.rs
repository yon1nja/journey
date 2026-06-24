use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use tempfile::NamedTempFile;

use crate::config;
use crate::models::{
    Index, IndexEntry, JourneyFile, JourneyStatus, RepoRef, WorktreeAttachment, WorktreeIndex,
};

pub const INDEX_FILE: &str = "index.yaml";
pub const WORKTREE_INDEX_FILE: &str = "worktree-index.yaml";
pub const JOURNEYS_DIR: &str = "journeys";
pub const JOURNEY_FILE: &str = "journey.yaml";
pub const JOURNAL_FILE: &str = "journal.jsonl";
pub const README_FILE: &str = "README.md";
pub const DOCS_DIR: &str = "docs";
pub const WORKTREES_DIR: &str = "worktrees";
pub const AGENTS_FILE: &str = "AGENTS.md";

const AGENTS_TEMPLATE: &str = r#"# AGENTS.md

You are working inside a Journey — a local context container for an engineering effort.

## This Journey

- **Config**: `journey.yaml` in this directory — contains id, title, description, status, and linked repos.
- **Journal**: `journal.jsonl` — append-only operational event log (link/unlink/status changes).
- **README**: `README.md` — optional user-owned top-level overview shown in Journey's interactive Details pane when present.
- **Docs**: `docs/` — user-owned Markdown files. You may create new docs here but never overwrite existing ones.
- **Worktrees**: `worktrees/` — symlinks to attached git worktrees (convenience view, not ownership).

Read `journey.yaml` first to understand what this effort is about.

## Journey CLI

The `journey` CLI manages this folder. Key commands:

- `journey status` — show this Journey's summary (run from this directory or any linked worktree).
- `journey link <repo-path>` — attach a git worktree to this Journey.
- `journey unlink <name>` — detach a worktree.
- `journey readme new` — create README.md if it does not already exist.
- `journey readme path` — print the absolute path to README.md.
- `journey doc new <name>` — create a new doc under `docs/`.
- `journey doc list` — list existing docs.
- `journey pause` / `journey resume` — lifecycle transitions.
- `journey list --non-interactive` — list all Journeys (for scripts/agents).

## Working with Config Files

- **`journey.yaml`**: YAML. Do not edit directly — use CLI commands to change status or link/unlink repos. You may read it freely to understand context.
- **`journal.jsonl`**: JSON Lines, append-only. Do not write to it directly — the CLI appends events automatically.
- **`README.md`**: User-owned Markdown. Use it for the Journey's top-level overview. Create it through `journey readme new` when missing; do not overwrite existing content.
- **`docs/*.md`**: You may create and edit these freely. Use them for investigation notes, plans, decisions, or any documentation relevant to the effort.

## Context Resolution

The CLI resolves which Journey you mean by:
1. Explicit `--journey <id>` flag.
2. Walking up from cwd looking for `journey.yaml`.
3. Matching cwd against the worktree index.
4. Failing with a clear error.

When working inside this folder or any linked worktree, commands resolve automatically.

## Constraints

- Journey is a context container, not a task manager. Do not create task-tracking files, checkpoints, or generated handoff documents.
- Git owns version control. Do not stash, snapshot, or capture dirty state through Journey.
- One worktree can be attached to only one active/paused Journey at a time.
"#;

#[derive(Debug, Clone)]
pub struct JourneyContext {
    pub home: PathBuf,
    pub path: PathBuf,
    pub journey: JourneyFile,
}

pub fn journey_home() -> Result<PathBuf> {
    if let Some(home) = env::var_os("JOURNEY_HOME") {
        return Ok(PathBuf::from(home));
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow!("could not determine home directory"))?;
    Ok(home.join(".journey"))
}

pub fn ensure_home(home: &Path) -> Result<()> {
    fs::create_dir_all(home.join(JOURNEYS_DIR))
        .with_context(|| format!("failed to create {}", home.join(JOURNEYS_DIR).display()))?;
    if !home.join(config::CONFIG_FILE).exists() {
        write_string_atomic(&home.join(config::CONFIG_FILE), config::DEFAULT_CONFIG_TOML)?;
    }
    if !home.join(INDEX_FILE).exists() {
        write_yaml_atomic(&home.join(INDEX_FILE), &Index::default())?;
    }
    if !home.join(WORKTREE_INDEX_FILE).exists() {
        write_yaml_atomic(&home.join(WORKTREE_INDEX_FILE), &WorktreeIndex::default())?;
    }
    Ok(())
}

pub fn journey_dir(home: &Path, id: &str) -> PathBuf {
    home.join(JOURNEYS_DIR).join(id)
}

pub fn load_index(home: &Path) -> Result<Index> {
    let path = home.join(INDEX_FILE);
    if !path.exists() {
        return Ok(Index::default());
    }
    read_yaml(&path)
}

pub fn save_index(home: &Path, index: &Index) -> Result<()> {
    write_yaml_atomic(&home.join(INDEX_FILE), index)
}

pub fn load_worktree_index(home: &Path) -> Result<WorktreeIndex> {
    let path = home.join(WORKTREE_INDEX_FILE);
    if !path.exists() {
        return Ok(WorktreeIndex::default());
    }
    read_yaml(&path)
}

pub fn save_worktree_index(home: &Path, index: &WorktreeIndex) -> Result<()> {
    write_yaml_atomic(&home.join(WORKTREE_INDEX_FILE), index)
}

pub fn load_journey(path: &Path) -> Result<JourneyFile> {
    read_yaml(&path.join(JOURNEY_FILE))
}

pub fn save_journey(path: &Path, journey: &JourneyFile) -> Result<()> {
    write_yaml_atomic(&path.join(JOURNEY_FILE), journey)
}

pub fn resolve_context(
    home: &Path,
    explicit_id: Option<&str>,
    cwd: &Path,
) -> Result<JourneyContext> {
    ensure_home(home)?;

    if let Some(id) = explicit_id {
        let path = journey_dir(home, id);
        if !path.join(JOURNEY_FILE).exists() {
            bail!("unknown Journey id: {id}");
        }
        let journey = load_journey(&path)?;
        return Ok(JourneyContext {
            home: home.to_path_buf(),
            path,
            journey,
        });
    }

    for dir in cwd.ancestors() {
        let candidate = dir.join(JOURNEY_FILE);
        if candidate.exists() {
            let journey = load_journey(dir)?;
            return Ok(JourneyContext {
                home: home.to_path_buf(),
                path: dir.to_path_buf(),
                journey,
            });
        }
    }

    if let Some(ctx) = resolve_context_from_worktree_index(home, cwd)? {
        return Ok(ctx);
    }

    bail!("not inside a Journey folder or attached worktree; pass an explicit Journey id");
}

pub fn create_journey(
    home: &Path,
    title: &str,
    description: Option<String>,
    now: &str,
) -> Result<JourneyContext> {
    ensure_home(home)?;

    let mut index = load_index(home)?;
    let id = allocate_id(&index, title);
    let path = journey_dir(home, &id);
    fs::create_dir_all(&path)
        .with_context(|| format!("failed to create Journey directory {}", path.display()))?;

    let journey = JourneyFile {
        id: id.clone(),
        title: title.to_string(),
        description: description.clone(),
        status: JourneyStatus::Active,
        created: now.to_string(),
        repos: Vec::new(),
    };

    save_journey(&path, &journey)?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path.join(JOURNAL_FILE))
        .with_context(|| format!("failed to create {}", path.join(JOURNAL_FILE).display()))?;
    fs::write(path.join(AGENTS_FILE), AGENTS_TEMPLATE)
        .with_context(|| format!("failed to create {}", path.join(AGENTS_FILE).display()))?;

    index.journeys.push(IndexEntry {
        id,
        title: title.to_string(),
        description,
        status: JourneyStatus::Active,
        updated: now.to_string(),
        repos: Vec::new(),
    });
    index.journeys.sort_by(|a, b| a.id.cmp(&b.id));
    save_index(home, &index)?;

    Ok(JourneyContext {
        home: home.to_path_buf(),
        path,
        journey,
    })
}

pub fn update_index_entry(home: &Path, journey: &JourneyFile, now: &str) -> Result<()> {
    let mut index = load_index(home)?;
    let repos: Vec<String> = journey.repos.iter().map(|repo| repo.name.clone()).collect();
    if let Some(entry) = index
        .journeys
        .iter_mut()
        .find(|entry| entry.id == journey.id)
    {
        entry.title = journey.title.clone();
        entry.description = journey.description.clone();
        entry.status = journey.status;
        entry.updated = now.to_string();
        entry.repos = repos;
    } else {
        index.journeys.push(IndexEntry {
            id: journey.id.clone(),
            title: journey.title.clone(),
            description: journey.description.clone(),
            status: journey.status,
            updated: now.to_string(),
            repos,
        });
    }
    index.journeys.sort_by(|a, b| a.id.cmp(&b.id));
    save_index(home, &index)
}

pub fn sync_worktree_link(journey_path: &Path, repo: &RepoRef) -> Result<()> {
    fs::create_dir_all(journey_path.join(WORKTREES_DIR))?;
    let link = journey_path.join(WORKTREES_DIR).join(&repo.name);

    if let Ok(meta) = fs::symlink_metadata(&link) {
        if meta.file_type().is_symlink() {
            let current = fs::read_link(&link)?;
            if current == repo.worktree {
                return Ok(());
            }
            fs::remove_file(&link)?;
        } else {
            bail!(
                "cannot create worktree link {}; path already exists and is not a symlink",
                link.display()
            );
        }
    }

    create_symlink(&repo.worktree, &link)
}

pub fn remove_worktree_link(journey_path: &Path, repo_name: &str) -> Result<()> {
    let link = journey_path.join(WORKTREES_DIR).join(repo_name);
    match fs::symlink_metadata(&link) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::remove_file(&link)
                .with_context(|| format!("failed to remove {}", link.display()))?;
        }
        Ok(_) => {
            bail!(
                "cannot remove worktree link {}; path exists and is not a symlink",
                link.display()
            );
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| format!("failed to inspect {}", link.display()))
        }
    }
    Ok(())
}

pub fn attach_worktree(
    home: &Path,
    journey_id: &str,
    repo_name: &str,
    worktree: &Path,
    attached_at: &str,
) -> Result<PathBuf> {
    let canonical_worktree = canonicalize_existing(worktree)?;
    let mut index = load_worktree_index(home)?;

    if let Some(existing) = index
        .attachments
        .iter()
        .find(|attachment| attachment.worktree == canonical_worktree)
    {
        bail!(
            "worktree {} is already attached to Journey `{}` as `{}`; unlink it first or use a different worktree",
            canonical_worktree.display(),
            existing.journey_id,
            existing.repo_name
        );
    }

    index.attachments.push(WorktreeAttachment {
        worktree: canonical_worktree.clone(),
        journey_id: journey_id.to_string(),
        repo_name: repo_name.to_string(),
        attached_at: attached_at.to_string(),
    });
    sort_worktree_index(&mut index);
    save_worktree_index(home, &index)?;
    Ok(canonical_worktree)
}

pub fn detach_worktree(home: &Path, journey_id: &str, repo_name: &str) -> Result<bool> {
    let mut index = load_worktree_index(home)?;
    let before = index.attachments.len();
    index.attachments.retain(|attachment| {
        !(attachment.journey_id == journey_id && attachment.repo_name == repo_name)
    });
    let removed = before != index.attachments.len();
    if removed {
        save_worktree_index(home, &index)?;
    }
    Ok(removed)
}

pub fn detach_journey_worktrees(home: &Path, journey_id: &str) -> Result<usize> {
    let mut index = load_worktree_index(home)?;
    let before = index.attachments.len();
    index
        .attachments
        .retain(|attachment| attachment.journey_id != journey_id);
    let removed = before - index.attachments.len();
    if removed > 0 {
        save_worktree_index(home, &index)?;
    }
    Ok(removed)
}

pub fn attach_journey_worktrees(
    home: &Path,
    journey: &JourneyFile,
    attached_at: &str,
) -> Result<usize> {
    let mut index = load_worktree_index(home)?;
    let mut added = 0;

    for repo in &journey.repos {
        let canonical_worktree = canonicalize_existing(&repo.worktree)?;
        if let Some(existing) = index
            .attachments
            .iter()
            .find(|attachment| attachment.worktree == canonical_worktree)
        {
            if existing.journey_id == journey.id && existing.repo_name == repo.name {
                continue;
            }
            bail!(
                "worktree {} is already attached to Journey `{}` as `{}`; unlink it first or use a different worktree",
                canonical_worktree.display(),
                existing.journey_id,
                existing.repo_name
            );
        }

        index.attachments.push(WorktreeAttachment {
            worktree: canonical_worktree,
            journey_id: journey.id.clone(),
            repo_name: repo.name.clone(),
            attached_at: attached_at.to_string(),
        });
        added += 1;
    }

    if added > 0 {
        sort_worktree_index(&mut index);
        save_worktree_index(home, &index)?;
    }
    Ok(added)
}

pub fn rebuild_worktree_index(home: &Path, attached_at: &str) -> Result<WorktreeIndex> {
    ensure_home(home)?;
    let index = load_index(home)?;
    let mut worktree_index = WorktreeIndex::default();

    for entry in index
        .journeys
        .iter()
        .filter(|entry| matches!(entry.status, JourneyStatus::Active | JourneyStatus::Paused))
    {
        let journey_path = journey_dir(home, &entry.id);
        if !journey_path.join(JOURNEY_FILE).exists() {
            continue;
        }
        let journey = load_journey(&journey_path)?;
        for repo in &journey.repos {
            let canonical_worktree = match canonicalize_existing(&repo.worktree) {
                Ok(path) => path,
                Err(_) => continue,
            };
            if let Some(existing) = worktree_index
                .attachments
                .iter()
                .find(|attachment| attachment.worktree == canonical_worktree)
            {
                bail!(
                    "cannot rebuild worktree index: {} is linked by Journey `{}` as `{}` and Journey `{}` as `{}`",
                    canonical_worktree.display(),
                    existing.journey_id,
                    existing.repo_name,
                    journey.id,
                    repo.name
                );
            }
            worktree_index.attachments.push(WorktreeAttachment {
                worktree: canonical_worktree,
                journey_id: journey.id.clone(),
                repo_name: repo.name.clone(),
                attached_at: attached_at.to_string(),
            });
        }
    }

    sort_worktree_index(&mut worktree_index);
    save_worktree_index(home, &worktree_index)?;
    Ok(worktree_index)
}

pub fn doc_path(journey_path: &Path, name: &str) -> Result<PathBuf> {
    let name = normalize_doc_name(name)?;
    Ok(journey_path.join(DOCS_DIR).join(name))
}

fn resolve_context_from_worktree_index(home: &Path, cwd: &Path) -> Result<Option<JourneyContext>> {
    let Ok(canonical_cwd) = canonicalize_existing(cwd) else {
        return Ok(None);
    };
    let index = load_worktree_index(home)?;
    let mut matches = index
        .attachments
        .iter()
        .filter(|attachment| canonical_cwd.starts_with(&attachment.worktree))
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| {
        b.worktree
            .components()
            .count()
            .cmp(&a.worktree.components().count())
    });

    for attachment in matches {
        let path = journey_dir(home, &attachment.journey_id);
        if path.join(JOURNEY_FILE).exists() {
            let journey = load_journey(&path)?;
            return Ok(Some(JourneyContext {
                home: home.to_path_buf(),
                path,
                journey,
            }));
        }
    }

    Ok(None)
}

pub fn canonicalize_existing(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("failed to canonicalize {}", path.display()))
}

fn sort_worktree_index(index: &mut WorktreeIndex) {
    index.attachments.sort_by(|a, b| {
        a.worktree
            .cmp(&b.worktree)
            .then_with(|| a.journey_id.cmp(&b.journey_id))
            .then_with(|| a.repo_name.cmp(&b.repo_name))
    });
}

pub fn normalize_doc_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("doc name cannot be empty");
    }

    let mut filename = trimmed.to_string();
    if !filename.ends_with(".md") {
        filename.push_str(".md");
    }

    let path = Path::new(&filename);
    if path.is_absolute() {
        bail!("doc name must be relative");
    }

    let components: Vec<Component<'_>> = path.components().collect();
    if components.len() != 1 || !matches!(components[0], Component::Normal(_)) {
        bail!("doc name must not contain path separators");
    }

    Ok(filename)
}

pub fn write_string_atomic(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        let mut tmp = NamedTempFile::new_in(parent)?;
        tmp.write_all(content.as_bytes())?;
        tmp.flush()?;
        tmp.persist(path)
            .map_err(|err| anyhow!("failed to persist {}: {}", path.display(), err.error))?;
        Ok(())
    } else {
        bail!("path has no parent: {}", path.display());
    }
}

pub fn write_yaml_atomic<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let content = serde_yaml::to_string(value)?;
    write_string_atomic(path, &content)
}

pub fn read_yaml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let mut content = String::new();
    fs::File::open(path)
        .with_context(|| format!("failed to open {}", path.display()))?
        .read_to_string(&mut content)?;
    serde_yaml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

fn allocate_id(index: &Index, title: &str) -> String {
    let base = slugify(title);
    if !index.journeys.iter().any(|entry| entry.id == base) {
        return base;
    }

    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !index.journeys.iter().any(|entry| entry.id == candidate) {
            return candidate;
        }
    }

    unreachable!("unbounded suffix loop must return")
}

pub fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }

    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "journey".to_string()
    } else {
        out
    }
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link).with_context(|| {
        format!(
            "failed to symlink {} -> {}",
            link.display(),
            target.display()
        )
    })
}

#[cfg(windows)]
fn create_symlink(target: &Path, link: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(target, link).with_context(|| {
        format!(
            "failed to symlink {} -> {}",
            link.display(),
            target.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugifies_titles() {
        assert_eq!(
            slugify("Investigate Auth Failures"),
            "investigate-auth-failures"
        );
        assert_eq!(slugify("  !!  "), "journey");
        assert_eq!(slugify("Review PR-1234"), "review-pr-1234");
    }

    #[test]
    fn rejects_nested_doc_names() {
        assert!(normalize_doc_name("design").is_ok());
        assert!(normalize_doc_name("design.md").is_ok());
        assert!(normalize_doc_name("../design").is_err());
        assert!(normalize_doc_name("plans/design").is_err());
    }

    #[test]
    fn ensure_home_creates_default_config() {
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path().join("journey-home");

        ensure_home(&home).unwrap();

        let config = fs::read_to_string(home.join(config::CONFIG_FILE)).unwrap();
        assert!(config.contains("[shortcuts]"));
        assert!(config.contains("open_claude = \"c\""));
        assert!(config.contains("normal_mode = \"esc\""));
    }
}
