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
    Done,
    Archived,
    Abandoned,
}

impl JourneyStatus {
    /// Open Journeys may own worktrees; closed ones release them.
    pub fn is_open(self) -> bool {
        matches!(self, JourneyStatus::Active | JourneyStatus::Paused)
    }
}

impl fmt::Display for JourneyStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            JourneyStatus::Active => "active",
            JourneyStatus::Paused => "paused",
            JourneyStatus::Done => "done",
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
            "done" => Ok(JourneyStatus::Done),
            "archived" => Ok(JourneyStatus::Archived),
            "abandoned" => Ok(JourneyStatus::Abandoned),
            other => Err(format!("unknown journey status: {other}")),
        }
    }
}

/// A side-quest is a short-lived kind of Journey. Promoting one only changes its kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JourneyKind {
    #[default]
    Journey,
    SideQuest,
}

impl JourneyKind {
    pub fn is_journey(&self) -> bool {
        *self == JourneyKind::Journey
    }
}

impl fmt::Display for JourneyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            JourneyKind::Journey => "journey",
            JourneyKind::SideQuest => "side-quest",
        })
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
    #[serde(default, skip_serializing_if = "JourneyKind::is_journey")]
    pub kind: JourneyKind,
    pub updated: String,
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
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
    #[serde(default, skip_serializing_if = "JourneyKind::is_journey")]
    pub kind: JourneyKind,
    pub status: JourneyStatus,
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Ids of related Journeys. Links are stored by id, never by path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
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
    Close {
        outcome: String,
    },
    Relate {
        journey: String,
    },
    Unrelate {
        journey: String,
    },
    Promote,
    #[serde(other)]
    Legacy,
}
