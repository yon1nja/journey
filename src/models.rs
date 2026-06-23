use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum JourneyStatus {
    Active,
    Paused,
    Archived,
    Abandoned,
}

impl fmt::Display for JourneyStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            JourneyStatus::Active => "active",
            JourneyStatus::Paused => "paused",
            JourneyStatus::Archived => "archived",
            JourneyStatus::Abandoned => "abandoned",
        };
        f.write_str(value)
    }
}

impl FromStr for JourneyStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(JourneyStatus::Active),
            "paused" => Ok(JourneyStatus::Paused),
            "archived" => Ok(JourneyStatus::Archived),
            "abandoned" => Ok(JourneyStatus::Abandoned),
            other => Err(format!("unknown journey status: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Index {
    #[serde(default)]
    pub journeys: Vec<IndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: JourneyStatus,
    pub updated: String,
    #[serde(default)]
    pub repos: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorktreeIndex {
    #[serde(default)]
    pub attachments: Vec<WorktreeAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeAttachment {
    pub worktree: PathBuf,
    pub journey_id: String,
    pub repo_name: String,
    pub attached_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JourneyFile {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: JourneyStatus,
    pub created: String,
    #[serde(default)]
    pub repos: Vec<RepoRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoRef {
    pub name: String,
    pub root: PathBuf,
    pub worktree: PathBuf,
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub seq: u64,
    pub ts: String,
    pub session: String,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    LinkRepo {
        name: String,
        root: PathBuf,
        worktree: PathBuf,
        branch: String,
    },
    UnlinkRepo {
        name: String,
        root: PathBuf,
        worktree: PathBuf,
        branch: String,
    },
    StatusChange {
        status: JourneyStatus,
    },
    #[serde(other)]
    Legacy,
}
