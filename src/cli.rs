use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::models::JourneyStatus;

#[derive(Debug, Parser)]
#[command(name = "journey")]
#[command(about = "Context persistence for engineering efforts")]
#[command(after_help = "Run `journey` with no subcommand to start the interactive Journey app.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Create a new Journey without opening the interactive app.
    New(NewArgs),
    /// Link a git repository or worktree to the current Journey.
    Link {
        repo_path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        journey: Option<String>,
    },
    /// Unlink a repository/worktree from the current Journey.
    Unlink {
        repo_name: String,
        #[arg(long)]
        journey: Option<String>,
    },
    /// Mark a Journey active.
    Resume(ContextIdArgs),
    /// List known Journeys.
    List {
        /// Initial status filter for the interactive UI; hard filter with --non-interactive.
        #[arg(long)]
        status: Option<JourneyStatus>,
        /// Print table output instead of opening the interactive UI.
        #[arg(long)]
        non_interactive: bool,
    },
    /// Print shell integration for interactive directory changes.
    #[command(name = "shell-init")]
    ShellInit,
    /// Print a one-screen Journey summary.
    Status { id: Option<String> },
    /// Manage Journey-local docs.
    Doc {
        #[command(subcommand)]
        command: DocCommands,
    },
    /// Manage a Journey README.md.
    Readme {
        #[command(subcommand)]
        command: ReadmeCommands,
    },
    /// Inspect or repair Journey indexes.
    Doctor {
        /// Rebuild the worktree attachment index from active/paused Journeys.
        #[arg(long)]
        repair: bool,
    },
    /// Mark a Journey paused.
    Pause(ContextIdArgs),
    /// Mark a Journey archived.
    Archive(ContextIdArgs),
    /// Mark a Journey abandoned.
    Abandon(ContextIdArgs),
}

#[derive(Debug, Args)]
pub struct NewArgs {
    #[arg(required = true, num_args = 1..)]
    pub text: Vec<String>,
    #[arg(short, long)]
    pub description: Option<String>,
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

#[derive(Debug, Subcommand)]
pub enum ReadmeCommands {
    /// Create README.md in the Journey folder.
    New {
        #[arg(long)]
        journey: Option<String>,
    },
    /// Print the absolute path to README.md.
    Path {
        #[arg(long)]
        journey: Option<String>,
    },
}

pub fn join_words(words: &[String]) -> String {
    words.join(" ")
}
