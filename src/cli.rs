use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::models::JourneyStatus;

#[derive(Debug, Parser)]
#[command(name = "journey")]
#[command(about = "Durable memory for engineering work, operated by agents")]
#[command(after_help = "Agents: run `journey guide` to learn what Journey is and how to use it.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Print the agent guide: what Journey is, when to use it, and how.
    Guide,
    /// Create a new Journey.
    New(NewArgs),
    /// Append a quick capture to a Journey-local Markdown doc.
    Capture(CaptureArgs),
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
    /// List Journeys as tab-separated rows: id, status, updated, repos.
    List {
        /// Only list Journeys with this status (defaults to active).
        #[arg(long)]
        status: Option<JourneyStatus>,
        /// Accepted for compatibility; output is always non-interactive.
        #[arg(long, hide = true)]
        non_interactive: bool,
    },
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
    /// Close a finished Journey with an outcome and mark it done.
    ///
    /// Releases worktree ownership. The worktrees stay on disk unless
    /// --remove-worktrees is passed.
    Close(CloseArgs),
    /// Create, link, or remove git worktrees for a Journey.
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommands,
    },
}

#[derive(Debug, Args)]
pub struct CloseArgs {
    pub id: Option<String>,
    /// One line describing how the effort ended.
    #[arg(long)]
    pub outcome: String,
    /// Also remove the linked worktrees from disk. Refuses if any has uncommitted changes.
    #[arg(long)]
    pub remove_worktrees: bool,
}

#[derive(Debug, Subcommand)]
pub enum WorktreeCommands {
    /// Create a worktree for a branch and link it to the Journey.
    ///
    /// Creates the branch if it does not exist. If the branch is already
    /// checked out in a worktree, links that worktree instead.
    New {
        /// Branch to check out in the new worktree.
        #[arg(long)]
        branch: String,
        /// Start point for a new branch (defaults to the repo's HEAD).
        #[arg(long)]
        from: Option<String>,
        /// Repository to create the worktree in (defaults to the current directory).
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Worktree location (defaults to <repo>/.worktrees/<branch>).
        #[arg(long)]
        path: Option<PathBuf>,
        /// Name to link the worktree under (defaults to the worktree folder name).
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        journey: Option<String>,
    },
    /// Remove a linked worktree from disk and unlink it.
    ///
    /// Refuses to remove the main working tree or a worktree with uncommitted changes.
    Remove {
        name: String,
        #[arg(long)]
        journey: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct NewArgs {
    #[arg(required = true, num_args = 1..)]
    pub text: Vec<String>,
    #[arg(short, long)]
    pub description: Option<String>,
}

#[derive(Debug, Args)]
pub struct CaptureArgs {
    #[arg(num_args = 0..)]
    pub text: Vec<String>,
    #[arg(long)]
    pub journey: Option<String>,
    #[arg(long, default_value = "capture")]
    pub doc: String,
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
