//! Upgrades state left by earlier Journey versions.
//!
//! Every step is idempotent and only touches files Journey generated itself:
//! a file is replaced or removed only when it still matches, byte for byte, a
//! default that an earlier version wrote. Anything the user edited is kept.

use std::path::PathBuf;

use super::*;

/// Per-journey `AGENTS.md` defaults written before v0.2.0. They inlined a copy
/// of the guidance that is now out of date.
const LEGACY_AGENTS: &[&str] = &[
    include_str!("../legacy/agents-1.md"),
    include_str!("../legacy/agents-2.md"),
    include_str!("../legacy/agents-3.md"),
];

/// Default `config.toml` files written by earlier versions. The file only
/// configured the TUI, which was removed in v0.3.0.
const LEGACY_CONFIGS: &[&str] = &[
    include_str!("../legacy/config-1.toml"),
    include_str!("../legacy/config-2.toml"),
    include_str!("../legacy/config-3.toml"),
    include_str!("../legacy/config-4.toml"),
    include_str!("../legacy/config-5.toml"),
];

const CONFIG_FILE: &str = "config.toml";

#[derive(Debug)]
enum Step {
    /// Replace an unedited legacy `AGENTS.md` with the current pointer.
    UpdateAgents(PathBuf),
    /// Add the `CLAUDE.md` that journeys created before v0.2.0 lack.
    CreateClaudeMd(PathBuf),
    /// Remove an unedited default `config.toml`.
    RemoveConfig(PathBuf),
}

struct Plan {
    steps: Vec<Step>,
    /// Things the user may want to handle; never changed automatically.
    notes: Vec<String>,
}

pub(crate) fn migrate(home: &Path, dry_run: bool) -> Result<String> {
    if !dry_run {
        storage::ensure_home(home)?;
    }
    let plan = plan(home)?;

    let mut out = Vec::new();
    for step in &plan.steps {
        let (verb, done, path, reason) = match step {
            Step::UpdateAgents(path) => ("update", "updated", path, "old default guidance"),
            Step::CreateClaudeMd(path) => ("create", "created", path, "missing"),
            Step::RemoveConfig(path) => ("remove", "removed", path, "TUI settings, no longer used"),
        };
        let verb = if dry_run {
            format!("would {verb}")
        } else {
            apply(home, step)?;
            done.to_string()
        };
        out.push(format!("{verb}: {} ({reason})", path.display()));
    }
    if plan.steps.is_empty() {
        out.push("nothing to migrate".to_string());
    }
    for note in &plan.notes {
        out.push(format!("note: {note}"));
    }
    Ok(out.join("\n"))
}

/// Number of migration steps waiting to run. Used by `journey doctor`.
pub(crate) fn pending(home: &Path) -> Result<usize> {
    Ok(plan(home)?.steps.len())
}

fn plan(home: &Path) -> Result<Plan> {
    let mut steps = Vec::new();
    let mut notes = Vec::new();

    let config = home.join(CONFIG_FILE);
    if let Ok(content) = fs::read_to_string(&config) {
        if LEGACY_CONFIGS.contains(&content.as_str()) {
            steps.push(Step::RemoveConfig(config));
        } else {
            notes.push(format!(
                "{} has your edits and is kept; Journey no longer reads it",
                config.display()
            ));
        }
    }

    for path in journey_dirs(home)? {
        let agents = path.join(storage::AGENTS_FILE);
        if let Ok(content) = fs::read_to_string(&agents) {
            if is_legacy_pointer(&content) || LEGACY_AGENTS.contains(&content.as_str()) {
                steps.push(Step::UpdateAgents(agents));
            }
        }
        let claude_md = path.join(storage::CLAUDE_MD_FILE);
        if !claude_md.exists() {
            steps.push(Step::CreateClaudeMd(claude_md));
        }
    }

    let missing = storage::load_worktree_index(home)?
        .attachments
        .iter()
        .filter(|attachment| !attachment.worktree.exists())
        .count();
    if missing > 0 {
        notes.push(format!(
            "{missing} worktree attachments point to folders that no longer exist; run `journey doctor` to review them"
        ));
    }

    Ok(Plan { steps, notes })
}

fn apply(home: &Path, step: &Step) -> Result<()> {
    match step {
        Step::UpdateAgents(path) => {
            storage::write_string_atomic(path, &storage::agents_template(home))
        }
        Step::CreateClaudeMd(path) => {
            storage::write_string_atomic(path, storage::CLAUDE_MD_TEMPLATE)
        }
        Step::RemoveConfig(path) => {
            fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))
        }
    }
}

/// Whether `content` is the per-journey `AGENTS.md` that v0.2.0 wrote. It
/// embeds the state root's path, which may have moved since, so any path to a
/// `JOURNEY-AGENTS.md` matches.
fn is_legacy_pointer(content: &str) -> bool {
    content
        .strip_prefix("# AGENTS.md\n\n@")
        .and_then(|rest| rest.split_once('\n'))
        .is_some_and(|(path, rest)| {
            path.ends_with(storage::JOURNEY_AGENTS_FILE)
                && rest == "\nRead the shared Journey guidance first, then read `journey.yaml` in this directory to understand this effort.\n"
        })
}

/// Every journey folder on disk, including any missing from the index.
fn journey_dirs(home: &Path) -> Result<Vec<PathBuf>> {
    let dir = home.join(storage::JOURNEYS_DIR);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut dirs = fs::read_dir(&dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(storage::JOURNEY_FILE).is_file())
        .collect::<Vec<_>>();
    dirs.sort();
    Ok(dirs)
}
