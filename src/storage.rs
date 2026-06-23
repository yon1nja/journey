use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use tempfile::NamedTempFile;

use crate::models::{Index, IndexEntry, JourneyFile, JourneyStatus, RepoRef};

pub const INDEX_FILE: &str = "index.yaml";
pub const JOURNEYS_DIR: &str = "journeys";
pub const JOURNEY_FILE: &str = "journey.yaml";
pub const JOURNAL_FILE: &str = "journal.jsonl";
pub const NOW_FILE: &str = "NOW.md";
pub const DOCS_DIR: &str = "docs";
pub const WORKTREES_DIR: &str = "worktrees";

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
    if !home.join(INDEX_FILE).exists() {
        write_yaml_atomic(&home.join(INDEX_FILE), &Index::default())?;
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

    bail!("not inside a Journey folder; pass an explicit Journey id");
}

pub fn create_journey(home: &Path, title: &str, now: &str) -> Result<JourneyContext> {
    ensure_home(home)?;

    let mut index = load_index(home)?;
    let id = allocate_id(&index, title);
    let path = journey_dir(home, &id);
    fs::create_dir_all(&path)
        .with_context(|| format!("failed to create Journey directory {}", path.display()))?;

    let journey = JourneyFile {
        id: id.clone(),
        title: title.to_string(),
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

    index.journeys.push(IndexEntry {
        id,
        title: title.to_string(),
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
        entry.status = journey.status;
        entry.updated = now.to_string();
        entry.repos = repos;
    } else {
        index.journeys.push(IndexEntry {
            id: journey.id.clone(),
            title: journey.title.clone(),
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

pub fn doc_path(journey_path: &Path, name: &str) -> Result<PathBuf> {
    let name = normalize_doc_name(name)?;
    Ok(journey_path.join(DOCS_DIR).join(name))
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
}
