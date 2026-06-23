use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::git;
use crate::models::{EventKind, EventRecord, JourneyFile};
use crate::storage::{self, DOCS_DIR, NOW_FILE};

pub fn write_now(journey_path: &Path, journey: &JourneyFile, events: &[EventRecord]) -> Result<()> {
    let content = render_now(journey_path, journey, events);
    storage::write_string_atomic(&journey_path.join(NOW_FILE), &content)
}

pub fn render_now(journey_path: &Path, journey: &JourneyFile, events: &[EventRecord]) -> String {
    let mut out = String::new();
    out.push_str("<!-- GENERATED - do not edit. Use journey note/decide/ask/next. -->\n\n");
    out.push_str(&format!("# {}\n\n", journey.title));
    out.push_str(&format!("- id: `{}`\n", journey.id));
    out.push_str(&format!("- status: `{}`\n", journey.status));
    out.push_str(&format!("- created: `{}`\n\n", journey.created));

    render_environment(&mut out, journey, events);
    render_next_actions(&mut out, events);
    render_questions(&mut out, events);
    render_decisions(&mut out, events);
    render_commands(&mut out, events);
    render_docs(&mut out, journey_path);

    out
}

fn render_environment(out: &mut String, journey: &JourneyFile, events: &[EventRecord]) {
    out.push_str("## Current Environment\n\n");
    let latest = events.iter().rev().find_map(|event| match &event.kind {
        EventKind::Checkpoint { message, repos } => Some((event, message, repos)),
        _ => None,
    });

    let Some((event, message, repos)) = latest else {
        out.push_str("No checkpoint recorded yet.\n\n");
        return;
    };

    out.push_str(&format!("- checkpoint: #{} at `{}`", event.seq, event.ts));
    if let Some(message) = message {
        out.push_str(&format!(" - {message}"));
    }
    out.push_str("\n\n");

    for repo in repos {
        out.push_str(&format!("### {}\n\n", repo.name));
        out.push_str(&format!("- branch: `{}`\n", repo.branch));
        out.push_str(&format!("- head: `{}`\n", short_sha(&repo.head)));
        if let Some(upstream) = &repo.upstream {
            out.push_str(&format!(
                "- upstream: `{}` (ahead {}, behind {})\n",
                upstream, repo.ahead, repo.behind
            ));
        } else {
            out.push_str("- upstream: none\n");
        }
        out.push_str(&format!("- tracked dirty: `{}`\n", repo.tracked_dirty));
        if let Some(snapshot_ref) = &repo.dirty_snapshot_ref {
            out.push_str(&format!("- dirty snapshot: `{snapshot_ref}`\n"));
            if let Some(repo_ref) = journey
                .repos
                .iter()
                .find(|candidate| candidate.name == repo.name)
            {
                match git::diff_stat(&repo_ref.worktree, &repo.head, snapshot_ref) {
                    Ok(stat) => {
                        out.push_str("\n```text\n");
                        out.push_str(&stat);
                        out.push_str("\n```\n");
                    }
                    Err(err) => {
                        out.push_str(&format!("- snapshot stat unavailable: {err}\n"));
                    }
                }
            }
        }
        if !repo.untracked_files.is_empty() {
            out.push_str(&format!(
                "- untracked files recorded, not snapshotted: {}\n",
                repo.untracked_files.len()
            ));
            for file in repo.untracked_files.iter().take(5) {
                out.push_str(&format!("  - `{file}`\n"));
            }
            if repo.untracked_files.len() > 5 {
                out.push_str(&format!(
                    "  - ... {} more\n",
                    repo.untracked_files.len() - 5
                ));
            }
        }
        out.push('\n');
    }
}

fn render_next_actions(out: &mut String, events: &[EventRecord]) {
    out.push_str("## Next Actions\n\n");
    let latest = events.iter().rev().find_map(|event| match &event.kind {
        EventKind::NextActions { items } => Some((event, items)),
        _ => None,
    });

    let Some((event, items)) = latest else {
        out.push_str("No next actions recorded.\n\n");
        return;
    };

    let stale_checkpoints = events
        .iter()
        .filter(|candidate| candidate.seq > event.seq)
        .filter(|candidate| matches!(candidate.kind, EventKind::Checkpoint { .. }))
        .count();
    out.push_str(&format!(
        "_Last set {}",
        age_phrase(&event.ts).unwrap_or_else(|| format!("at {}", event.ts))
    ));
    if stale_checkpoints > 0 {
        out.push_str(&format!(", {stale_checkpoints} checkpoints stale"));
    }
    out.push_str("._\n\n");

    for item in items {
        out.push_str(&format!("- {item}\n"));
    }
    out.push('\n');
}

fn render_questions(out: &mut String, events: &[EventRecord]) {
    out.push_str("## Open Questions\n\n");
    let mut questions = BTreeMap::new();
    for event in events {
        match &event.kind {
            EventKind::QuestionOpen { qid, text } => {
                questions.insert(qid.clone(), text.clone());
            }
            EventKind::QuestionResolve { qid, .. } => {
                questions.remove(qid);
            }
            _ => {}
        }
    }

    if questions.is_empty() {
        out.push_str("No open questions.\n\n");
        return;
    }

    for (qid, text) in questions {
        out.push_str(&format!("- `{qid}` {text}\n"));
    }
    out.push('\n');
}

fn render_decisions(out: &mut String, events: &[EventRecord]) {
    out.push_str("## Recent Decisions\n\n");
    let decisions: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Decision { did, text, because } => Some((event, did, text, because)),
            _ => None,
        })
        .rev()
        .take(5)
        .collect();

    if decisions.is_empty() {
        out.push_str("No decisions recorded.\n\n");
        return;
    }

    for (event, did, text, because) in decisions.into_iter().rev() {
        out.push_str(&format!("- `{did}` {text} _({})_", event.ts));
        if let Some(because) = because {
            out.push_str(&format!(" because {because}"));
        }
        out.push('\n');
    }
    out.push('\n');
}

fn render_commands(out: &mut String, events: &[EventRecord]) {
    out.push_str("## Last Commands\n\n");
    let commands: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Command { cmd, exit, cwd } => Some((event, cmd, exit, cwd)),
            _ => None,
        })
        .rev()
        .take(5)
        .collect();

    if commands.is_empty() {
        out.push_str("No commands recorded.\n\n");
        return;
    }

    for (event, cmd, exit, cwd) in commands.into_iter().rev() {
        out.push_str(&format!(
            "- `{cmd}` exit `{exit}` in `{}` _({})_\n",
            cwd.display(),
            event.ts
        ));
    }
    out.push('\n');
}

fn render_docs(out: &mut String, journey_path: &Path) {
    let docs_dir = journey_path.join(DOCS_DIR);
    let Ok(entries) = fs::read_dir(&docs_dir) else {
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
        return;
    }

    out.push_str("## Docs\n\n");
    for doc in docs {
        out.push_str(&format!("- `docs/{doc}`\n"));
    }
    out.push('\n');
}

fn short_sha(value: &str) -> String {
    value.chars().take(12).collect()
}

fn age_phrase(ts: &str) -> Option<String> {
    let then = OffsetDateTime::parse(ts, &Rfc3339).ok()?;
    let now = OffsetDateTime::now_utc();
    let duration = now - then;
    if duration.is_negative() {
        return Some(format!("at {ts}"));
    }

    let seconds = duration.whole_seconds();
    let phrase = if seconds < 60 {
        "just now".to_string()
    } else if seconds < 60 * 60 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 60 * 60 * 24 {
        format!("{}h ago", seconds / 60 / 60)
    } else {
        format!("{}d ago", seconds / 60 / 60 / 24)
    };
    Some(phrase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EventRecord, JourneyStatus};

    #[test]
    fn renders_open_questions_and_next_actions() {
        let journey = JourneyFile {
            id: "test".to_string(),
            title: "Test Journey".to_string(),
            status: JourneyStatus::Active,
            created: "2026-01-01T00:00:00Z".to_string(),
            repos: Vec::new(),
        };
        let events = vec![
            EventRecord {
                seq: 1,
                ts: "2026-01-01T00:00:00Z".to_string(),
                session: "test".to_string(),
                kind: EventKind::QuestionOpen {
                    qid: "q1".to_string(),
                    text: "What broke?".to_string(),
                },
            },
            EventRecord {
                seq: 2,
                ts: "2026-01-01T00:00:00Z".to_string(),
                session: "test".to_string(),
                kind: EventKind::NextActions {
                    items: vec!["Reproduce failure".to_string()],
                },
            },
        ];

        let rendered = render_now(Path::new("/tmp/does-not-exist"), &journey, &events);
        assert!(rendered.contains("`q1` What broke?"));
        assert!(rendered.contains("Reproduce failure"));
    }
}
