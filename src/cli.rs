use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::models::JourneyStatus;

#[derive(Debug, Parser)]
#[command(name = "journey")]
#[command(about = "Context persistence for engineering efforts")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Create a new Journey.
    New(TextArgs),
    /// Link a git repository or worktree to the current Journey.
    Link {
        repo_path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        journey: Option<String>,
    },
    /// Capture git state for linked repos and regenerate NOW.md.
    Checkpoint {
        #[arg(short, long)]
        message: Option<String>,
        #[arg(long)]
        journey: Option<String>,
    },
    /// Resume a Journey and print derived context.
    Resume {
        id: Option<String>,
        #[arg(long)]
        apply: bool,
    },
    /// List known Journeys.
    List {
        #[arg(long)]
        status: Option<JourneyStatus>,
    },
    /// Print a one-screen Journey summary.
    Status { id: Option<String> },
    /// Append a note event.
    Note(ContextTextArgs),
    /// Append a decision event.
    Decide(DecideArgs),
    /// Open a question.
    Ask(ContextTextArgs),
    /// Resolve a question.
    Resolve(ResolveArgs),
    /// Replace next actions.
    Next(NextArgs),
    /// Manage Journey-local docs.
    Doc {
        #[command(subcommand)]
        command: DocCommands,
    },
    /// Mark a Journey paused.
    Pause(ContextIdArgs),
    /// Mark a Journey archived.
    Archive(ContextIdArgs),
    /// Mark a Journey abandoned.
    Abandon(ContextIdArgs),
}

#[derive(Debug, Args)]
pub struct TextArgs {
    #[arg(required = true, num_args = 1..)]
    pub text: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ContextTextArgs {
    #[arg(required = true, num_args = 1..)]
    pub text: Vec<String>,
    #[arg(long)]
    pub journey: Option<String>,
}

#[derive(Debug, Args)]
pub struct DecideArgs {
    #[arg(required = true, num_args = 1..)]
    pub text: Vec<String>,
    #[arg(long)]
    pub because: Option<String>,
    #[arg(long)]
    pub journey: Option<String>,
}

#[derive(Debug, Args)]
pub struct ResolveArgs {
    pub qid: String,
    #[arg(required = true, num_args = 1..)]
    pub answer: Vec<String>,
    #[arg(long)]
    pub journey: Option<String>,
}

#[derive(Debug, Args)]
pub struct NextArgs {
    #[arg(required = true, num_args = 1..)]
    pub items: Vec<String>,
    #[arg(long)]
    pub journey: Option<String>,
}

#[derive(Debug, Args)]
pub struct ContextIdArgs {
    pub id: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum DocCommands {
    /// Create docs/<name>.md.
    New {
        name: String,
        #[arg(long)]
        journey: Option<String>,
    },
    /// List Journey docs.
    List {
        #[arg(long)]
        journey: Option<String>,
    },
    /// Print the absolute path to a Journey doc.
    Path {
        name: String,
        #[arg(long)]
        journey: Option<String>,
    },
}

pub fn join_words(words: &[String]) -> String {
    words.join(" ")
}
