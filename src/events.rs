use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use anyhow::{Context, Result};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::models::{EventKind, EventRecord};
use crate::storage::JOURNAL_FILE;

pub fn now_rfc3339() -> Result<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

pub fn session_id() -> String {
    env::var("JOURNEY_SESSION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| env::var("USER").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "local".to_string())
}

pub fn read_events(journey_path: &Path) -> Result<Vec<EventRecord>> {
    let path = journey_path.join(JOURNAL_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = fs::File::open(&path)
        .with_context(|| format!("failed to open journal {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();

    for (idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("failed to read line {}", idx + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<EventRecord>(&line)
            .with_context(|| format!("failed to parse journal line {}", idx + 1))?;
        events.push(event);
    }

    Ok(events)
}

pub fn append_event(journey_path: &Path, kind: EventKind) -> Result<EventRecord> {
    let mut events = read_events(journey_path)?;
    let seq = events.iter().map(|event| event.seq).max().unwrap_or(0) + 1;
    let record = EventRecord {
        seq,
        ts: now_rfc3339()?,
        session: session_id(),
        kind,
    };

    let path = journey_path.join(JOURNAL_FILE);
    let line = serde_json::to_string(&record)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open journal {}", path.display()))?;
    writeln!(file, "{line}")?;
    events.push(record.clone());
    Ok(record)
}

pub fn next_decision_id(events: &[EventRecord]) -> String {
    next_prefixed_id(events, "d", |kind| match kind {
        EventKind::Decision { did, .. } => Some(did.as_str()),
        _ => None,
    })
}

pub fn next_question_id(events: &[EventRecord]) -> String {
    next_prefixed_id(events, "q", |kind| match kind {
        EventKind::QuestionOpen { qid, .. } => Some(qid.as_str()),
        _ => None,
    })
}

fn next_prefixed_id<F>(events: &[EventRecord], prefix: &str, extract: F) -> String
where
    F: Fn(&EventKind) -> Option<&str>,
{
    let max = events
        .iter()
        .filter_map(|event| extract(&event.kind))
        .filter_map(|id| id.strip_prefix(prefix))
        .filter_map(|suffix| suffix.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}{}", max + 1)
}
